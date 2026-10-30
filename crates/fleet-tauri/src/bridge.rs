//! The native bridge: what the web UI asks of the machine it runs on, when it runs in the Tauri app. Each command
//! calls `fleet-client`, which holds the decisions the egui app made, so the two apps decide alike.
use fleet_api::{Effort, PermissionMode, PersonaList, ToolId, ToolList};
use fleet_client::config::{self, ClientConfig, GuiSettings, SettingsWriter};
use fleet_client::config::{CloseChoice, FleetDisplay, MonoFont, ThemeChoice, UiFont};
use fleet_client::daemon_control::{
    connect_or_start, find_daemon_binary, stop_daemon, Started, StopError,
};
use fleet_client::http::FleetClient;
use fleet_client::install::{install_argv, real_program, run_install, waiting_terminal};
use fleet_client::login::{self, is_loopback_url};
use fleet_client::shortcuts;
use fleet_client::update::{latest, releases_url, Release, UpdateState};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri_plugin_dialog::DialogExt;

/// Where this window's config lives and whose home it expands `~` against.
#[derive(Clone, Debug)]
pub struct Paths {
    pub config: PathBuf,
    pub home: PathBuf,
}

impl Paths {
    /// `--config PATH` when given, else the default config of the user running the app.
    pub fn from_args(args: impl IntoIterator<Item = String>) -> Paths {
        let home = fleet_fs::dirs::home_dir();
        let config = config::config_arg(args).unwrap_or_else(|| config::default_path(&home));
        Paths { config, home }
    }

    /// The config as it is now; a broken one gives the defaults, as egui's window does until Retry.
    /// egui's `gui.toml`, which both apps share.
    pub fn settings(&self) -> PathBuf {
        config::gui_settings_path(&self.home)
    }

    pub fn load(&self) -> ClientConfig {
        config::load(&self.config, &self.home)
            .unwrap_or_else(|_| config::defaults(&self.config, &self.home))
    }
}

#[tauri::command]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Whether the config file exists: without one, this is a first run.
#[tauri::command]
pub fn has_config(paths: tauri::State<'_, Paths>) -> bool {
    config::exists(&paths.config)
}

/// The daemon this window last connected to, as egui's `DaemonCtl` keeps it: a restart stops that one and the Daemon
/// tab shows it, whatever `config.toml` says since.
#[derive(Clone, Debug, PartialEq)]
pub struct Connected {
    pub listen: String,
    pub base_url: String,
    pub state_dir: PathBuf,
    /// The folders the daemon runs with: the config may hold others, written since, that wait for a restart.
    pub tasks_root: PathBuf,
    pub personas_dir: PathBuf,
}

impl Connected {
    fn of(cfg: &ClientConfig) -> Self {
        Connected {
            listen: cfg.listen.to_string(),
            base_url: cfg.base_url(),
            state_dir: cfg.state_dir.clone(),
            tasks_root: cfg.tasks_root.clone(),
            personas_dir: cfg.personas_dir.clone(),
        }
    }
}

#[derive(Default)]
struct OwnerState {
    pid: Option<u32>,
    connected: Option<Connected>,
}

/// The daemon this window started, if it did (only that one is stopped on close without asking), and the one it
/// talks to.
#[derive(Default, Clone)]
pub struct Owner(Arc<Mutex<OwnerState>>);

impl Owner {
    fn state(&self) -> std::sync::MutexGuard<'_, OwnerState> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn pid(&self) -> Option<u32> {
        self.state().pid
    }

    pub fn connected(&self) -> Option<Connected> {
        self.state().connected.clone()
    }
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StartReply {
    pub we_started: bool,
    /// The daemon's address: on port 0, the one it wrote to `daemon.addr` once up.
    pub url: String,
    /// The daemon is on this machine: terminals, installs and logins are offered only then.
    pub local: bool,
}

/// Connects to the daemon or starts it, as egui's window does: a config that can't be read stops here with its
/// reason, where the other commands fall back to the defaults.
pub fn start_with(
    paths: &Paths,
    owner: &Owner,
    find: impl FnOnce() -> Option<PathBuf>,
) -> Result<StartReply, String> {
    let cfg = config::load(&paths.config, &paths.home).map_err(|e| e.to_string())?;
    let started = connect_or_start(&cfg, find, START_WAIT).map_err(|e| e.to_string())?;
    own(owner, &started, &cfg);
    let url = cfg.base_url();
    Ok(StartReply {
        we_started: owner.pid().is_some(),
        local: is_loopback_url(&url),
        url,
    })
}

/// egui's wait for a daemon it starts to come up.
const START_WAIT: Duration = Duration::from_secs(10);

/// A daemon this window started is its own; one already running keeps the owner a previous start recorded (a Retry
/// that connects to a daemon already up).
fn own(owner: &Owner, started: &Started, cfg: &ClientConfig) {
    let mut state = owner.state();
    if let Started::StartedByUs(pid) = started {
        state.pid = Some(*pid);
    }
    state.connected = Some(Connected::of(cfg));
}

/// Blocks for up to the start's wait, so it runs off the async runtime's workers.
#[tauri::command]
pub async fn start_daemon(
    paths: tauri::State<'_, Paths>,
    owner: tauri::State<'_, Owner>,
) -> Result<StartReply, String> {
    let (paths, owner) = (paths.inner().clone(), owner.inner().clone());
    tauri::async_runtime::spawn_blocking(move || start_with(&paths, &owner, find_daemon_binary))
        .await
        .map_err(|e| e.to_string())?
}

/// Where the daemon keeps its state, shown on the failure screen.
#[tauri::command]
pub fn state_dir(paths: tauri::State<'_, Paths>) -> String {
    paths.load().state_dir.display().to_string()
}

/// The first run's fields as egui fills them, `~` for the home folder.
#[derive(Serialize, Debug, PartialEq)]
pub struct FirstRunDefaults {
    pub root: String,
    pub personas: String,
}

pub fn first_run_defaults_of(paths: &Paths) -> FirstRunDefaults {
    FirstRunDefaults {
        root: "~/fleet-tasks".into(),
        personas: config::shorten_home(&config::default_personas_dir(&paths.home), &paths.home),
    }
}

#[tauri::command]
pub fn first_run_defaults(paths: tauri::State<'_, Paths>) -> FirstRunDefaults {
    first_run_defaults_of(&paths)
}

/// egui's `save_first_run`: the folders checked and the tasks folder made, then the config written. Its errors are
/// egui's texts.
pub fn save_first_run(paths: &Paths, root: &str, personas: &str) -> Result<(), String> {
    let personas = match personas.trim() {
        "" => return Err("Enter a folder for the personas".into()),
        p => config::expand_home(p, &paths.home),
    };
    let dir = config::prepare_tasks_root(root, &paths.home)?;
    match config::write_initial(&paths.config, &dir, &personas, &paths.home) {
        Ok(()) => Ok(()),
        // Another instance or a hand edit got there first: keep its file and just start.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(format!("Cannot write {}: {e}", paths.config.display())),
    }
}

#[tauri::command]
pub fn write_initial_config(
    paths: tauri::State<'_, Paths>,
    root: String,
    personas: String,
) -> Result<(), String> {
    save_first_run(&paths, &root, &personas)
}

/// Where the folder dialog opens: the field's folder when it exists, else where the last pick was made, else home.
pub fn pick_start(typed: Option<&str>, settings: &Path, home: &Path) -> PathBuf {
    if let Some(dir) = typed
        .map(|t| config::expand_home(t.trim(), home))
        .filter(|d| d.is_dir())
    {
        return dir;
    }
    SettingsWriter::shared()
        .current(settings)
        .last_pick_dir
        .filter(|d| d.is_dir())
        .unwrap_or_else(|| home.to_path_buf())
}

/// Keeps the folder picked for the next dialog. Losing it only costs the next opening place.
pub fn remember_pick(settings: &Path, picked: &Path) {
    // egui's folder mode keeps the folder Select was clicked in: the one picked.
    let from = picked.to_path_buf();
    SettingsWriter::shared().update(
        settings,
        move |s: &mut GuiSettings| s.last_pick_dir = Some(from.clone()),
        |_| {},
    );
}

/// The OS's folder dialog. Blocks until it closes, so it runs off the async runtime's workers.
#[tauri::command]
pub async fn pick_folder(
    app: tauri::AppHandle,
    paths: tauri::State<'_, Paths>,
    start: Option<String>,
) -> Result<Option<String>, String> {
    let (settings, home) = (paths.settings(), paths.home.clone());
    tauri::async_runtime::spawn_blocking(move || {
        let from = pick_start(start.as_deref(), &settings, &home);
        let picked = app
            .dialog()
            .file()
            .set_directory(from)
            .blocking_pick_folder()?
            .into_path()
            .ok()?;
        remember_pick(&settings, &picked);
        Some(picked.display().to_string())
    })
    .await
    .map_err(|e| e.to_string())
}

/// The daemon this window talks to, for the commands that act on what it lists. Native actions run on this machine,
/// so they refuse a daemon elsewhere (the web offers them only for a local one).
fn local_daemon(paths: &Paths) -> Result<FleetClient, String> {
    let url = paths.load().base_url();
    if !is_loopback_url(&url) {
        return Err("The daemon is on another machine".into());
    }
    Ok(FleetClient::new(&url))
}

/// The command the daemon gives to install `tool`. The webview names the tool, never the command, so no page can
/// have the app run a command of its own.
pub fn install_command(list: &ToolList, tool: ToolId) -> Result<String, String> {
    list.tools
        .iter()
        .find(|t| t.tool == tool)
        .and_then(|t| t.error.as_ref()?.fix()?.install.clone())
        .ok_or_else(|| format!("{} needs no install", tool.name()))
}

/// egui's install seam on Linux: the daemon's command in a terminal that holds the call until its window closes.
#[tauri::command]
pub async fn install_tool(paths: tauri::State<'_, Paths>, tool: ToolId) -> Result<(), String> {
    let paths = paths.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let list = local_daemon(&paths)?.tools().map_err(|e| e.message)?;
        let command = install_command(&list, tool)?;
        let terminal = login::terminal().map(|t| waiting_terminal(t, real_program));
        let argv = install_argv(&command, fleet_api::Os::current(), None, terminal)
            .ok_or("No terminal was found to run the install in")?;
        let mut child =
            run_install(argv).map_err(|e| format!("Could not open the install window: {e}"))?;
        child.wait().map(drop).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Login {
    Claude,
    Gh,
}

/// The script a persona's login runs, built by fleet-client from the daemon's list, as egui builds it.
pub fn login_script(list: &PersonaList, id: &str, which: Login) -> Result<String, String> {
    let p = list
        .personas
        .iter()
        .find(|p| p.id == id)
        .ok_or("The persona is gone")?;
    Ok(match which {
        Login::Claude => {
            login::claude_login_script(p, &list.claude_login_command(), &list.tasks_root)
        }
        Login::Gh => login::gh_login_script(p, list.gh_login_program()),
    })
}

/// What a Log in… did: `script` is set when no terminal was found, for the user to run by hand.
#[derive(Serialize, Debug, PartialEq)]
pub struct LoginOpened {
    pub script: Option<String>,
}

#[tauri::command]
pub async fn run_login(
    paths: tauri::State<'_, Paths>,
    persona: String,
    which: Login,
) -> Result<LoginOpened, String> {
    let paths = paths.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let list = local_daemon(&paths)?.personas().map_err(|e| e.message)?;
        let script = login_script(&list, &persona, which)?;
        let Some(term) = login::terminal() else {
            return Ok(LoginOpened {
                script: Some(script),
            });
        };
        login::open_in_terminal(&term, &script)
            .map(|()| LoginOpened { script: None })
            .map_err(|e| format!("Could not open a terminal: {e}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Hands a token to the persona's gh. It lives only in this call: never logged, never kept.
#[tauri::command]
pub async fn save_pat(
    paths: tauri::State<'_, Paths>,
    persona: String,
    token: String,
) -> Result<(), String> {
    let paths = paths.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let list = local_daemon(&paths)?.personas().map_err(|e| e.message)?;
        let p = list
            .personas
            .iter()
            .find(|p| p.id == persona)
            .ok_or("The persona is gone")?;
        login::pat_login(p, list.gh_login_program(), &token)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The wizard's "Add an app menu shortcut": the app menu only, under the Tauri app's own entry.
#[tauri::command]
pub fn add_menu_shortcut() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let menu = shortcuts::Which {
        menu: true,
        desktop: false,
    };
    shortcuts::add_for(&shortcuts::places(), menu, &exe, shortcuts::TAURI)
        .map_err(|e| format!("Could not change the shortcuts: {e}"))
}

/// Settings' Shortcuts section: the Tauri app's own entries only; egui's are never read or touched.
#[derive(Serialize, Debug, PartialEq)]
pub struct ShortcutsReply {
    /// `none`, `current` (every entry points at this exe) or `stale` (one points elsewhere).
    pub state: &'static str,
    pub menu: bool,
    pub desktop: bool,
}

fn shortcuts_reply(places: &shortcuts::Places, exe: &Path) -> ShortcutsReply {
    match shortcuts::state(places, exe, shortcuts::TAURI) {
        shortcuts::ShortcutState::None => ShortcutsReply {
            state: "none",
            menu: false,
            desktop: false,
        },
        shortcuts::ShortcutState::Current { menu, desktop } => ShortcutsReply {
            state: "current",
            menu,
            desktop,
        },
        shortcuts::ShortcutState::Stale => {
            let found = shortcuts::existing(places, shortcuts::TAURI);
            ShortcutsReply {
                state: "stale",
                menu: found.menu,
                desktop: found.desktop,
            }
        }
    }
}

#[tauri::command]
pub fn shortcuts_state() -> Result<ShortcutsReply, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok(shortcuts_reply(&shortcuts::places(), &exe))
}

/// Add or Remove the ticked places, or Update the ones that exist to this exe (egui's `ShortcutsAction`).
#[tauri::command]
pub fn change_shortcuts(
    action: String,
    menu: bool,
    desktop: bool,
) -> Result<ShortcutsReply, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let places = shortcuts::places();
    let which = shortcuts::Which { menu, desktop };
    match action.as_str() {
        "add" => shortcuts::add_for(&places, which, &exe, shortcuts::TAURI),
        "remove" => shortcuts::remove(&places, which, shortcuts::TAURI),
        "update" => shortcuts::add_for(
            &places,
            shortcuts::existing(&places, shortcuts::TAURI),
            &exe,
            shortcuts::TAURI,
        ),
        _ => return Err(format!("no shortcut action {action}")),
    }
    .map_err(|e| e.to_string())?;
    Ok(shortcuts_reply(&places, &exe))
}

/// Takes the daemon's PATH for the terminals this app opens, when the daemon is on this machine (egui's
/// `adopt_path`).
#[tauri::command]
pub fn adopt_path(paths: tauri::State<'_, Paths>, path: String) {
    if is_loopback_url(&paths.load().base_url()) {
        fleet_fs::child::set_path(path.into());
    }
}

/// Open in terminal: the agent's attach command, looked up from the daemon by id, in a terminal on this machine.
#[tauri::command]
pub async fn open_terminal(paths: tauri::State<'_, Paths>, agent: String) -> Result<(), String> {
    let paths = paths.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let snapshot = local_daemon(&paths)?.snapshot().map_err(|e| e.message)?;
        let attach = snapshot
            .agents
            .iter()
            .find(|a| a.id.0 == agent)
            .and_then(|a| a.attach.clone())
            .ok_or("the agent is stopped")?;
        let term = login::terminal().ok_or("no terminal emulator found")?;
        login::open_in_terminal(&term, &login::shell_join(&attach))
            .map_err(|e| format!("Could not open a terminal: {e}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Settings' daemon settings as `config.toml` has them, for General's fields.
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DaemonSettings {
    /// `~`-shortened, as the fields show them.
    pub tasks_root: String,
    pub personas_dir: String,
    pub limits: LimitsDto,
    pub pr: PrDto,
    pub default_mode: PermissionMode,
    pub default_effort: Effort,
    /// What `~` stands for, so the page can tell a typed absolute path is the current folder.
    pub home: String,
    /// The folders the daemon runs with, `~`-shortened; `tasks_root` and `personas_dir` are the config's.
    pub running_tasks_root: String,
    pub running_personas_dir: String,
}

/// `config::Limits` over the bridge.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LimitsDto {
    pub max_breakdown_versions: u32,
    pub max_steps: u32,
    pub max_parallel_specialists: u32,
    pub max_pr_attempts: u32,
    /// None is no cap.
    pub max_concurrent_tasks: Option<u32>,
    pub max_active_products: Option<u32>,
}

impl From<config::Limits> for LimitsDto {
    fn from(l: config::Limits) -> Self {
        LimitsDto {
            max_breakdown_versions: l.max_breakdown_versions,
            max_steps: l.max_steps,
            max_parallel_specialists: l.max_parallel_specialists,
            max_pr_attempts: l.max_pr_attempts,
            max_concurrent_tasks: l.max_concurrent_tasks,
            max_active_products: l.max_active_products,
        }
    }
}

impl From<LimitsDto> for config::Limits {
    fn from(l: LimitsDto) -> Self {
        config::Limits {
            max_breakdown_versions: l.max_breakdown_versions,
            max_steps: l.max_steps,
            max_parallel_specialists: l.max_parallel_specialists,
            max_pr_attempts: l.max_pr_attempts,
            max_concurrent_tasks: l.max_concurrent_tasks,
            max_active_products: l.max_active_products,
        }
    }
}

/// `config::PrSettings` over the bridge; `gate_build` is `shared` or `clean`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PrDto {
    pub style: String,
    pub attribution: String,
    pub draft: bool,
    pub labels: Vec<String>,
    pub gate_build: String,
}

impl From<&config::PrSettings> for PrDto {
    fn from(p: &config::PrSettings) -> Self {
        PrDto {
            style: p.style.clone(),
            attribution: p.attribution.clone(),
            draft: p.draft,
            labels: p.labels.clone(),
            gate_build: p.gate_build.as_str().into(),
        }
    }
}

impl TryFrom<&PrDto> for config::PrSettings {
    type Error = String;
    fn try_from(p: &PrDto) -> Result<Self, String> {
        Ok(config::PrSettings {
            style: p.style.clone(),
            attribution: p.attribution.clone(),
            draft: p.draft,
            labels: p.labels.clone(),
            gate_build: p
                .gate_build
                .parse()
                .map_err(|()| format!("no gate build {:?}", p.gate_build))?,
        })
    }
}

pub fn daemon_settings_of(
    cfg: &ClientConfig,
    home: &Path,
    connected: Option<&Connected>,
) -> DaemonSettings {
    let (running_root, running_personas) = connected
        .map_or((&cfg.tasks_root, &cfg.personas_dir), |c| {
            (&c.tasks_root, &c.personas_dir)
        });
    DaemonSettings {
        running_tasks_root: config::shorten_home(running_root, home),
        running_personas_dir: config::shorten_home(running_personas, home),
        tasks_root: config::shorten_home(&cfg.tasks_root, home),
        personas_dir: config::shorten_home(&cfg.personas_dir, home),
        limits: cfg.limits.into(),
        pr: (&cfg.pr).into(),
        default_mode: cfg.default_mode,
        default_effort: cfg.default_effort,
        home: home.display().to_string(),
    }
}

/// A broken config reads as the defaults, as the other commands do; the daemon names the problem when it starts.
#[tauri::command]
pub fn read_daemon_settings(
    paths: tauri::State<'_, Paths>,
    owner: tauri::State<'_, Owner>,
) -> DaemonSettings {
    daemon_settings_of(&paths.load(), &paths.home, owner.connected().as_ref())
}

/// One of Settings' Saves, or Restart daemon (nothing to write). egui's `RestartFor`.
#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SettingsChange {
    TasksRoot {
        text: String,
    },
    PersonasDir {
        text: String,
    },
    Limits(LimitsDto),
    Pr(PrDto),
    AgentDefaults {
        mode: PermissionMode,
        effort: Effort,
    },
    Restart,
}

/// Writes `change` to `config.toml`, as egui's `start_restart` does before restarting. Its errors are egui's texts.
pub fn apply_change(paths: &Paths, change: &SettingsChange) -> Result<(), String> {
    let (config, home) = (&paths.config, &paths.home);
    let written =
        |r: std::io::Result<()>| r.map_err(|e| format!("Cannot write {}: {e}", config.display()));
    match change {
        SettingsChange::TasksRoot { text } => {
            let dir = config::prepare_tasks_root(text, home)?;
            written(config::set_tasks_root(config, &dir, home))
        }
        SettingsChange::PersonasDir { text } => {
            let dir = config::prepare_personas_dir(text, home)?;
            written(config::set_personas_dir(config, &dir, home))
        }
        SettingsChange::Limits(l) => written(config::set_limits(config, &(*l).into(), home)),
        SettingsChange::Pr(p) => written(config::set_pr(config, &p.try_into()?, home)),
        SettingsChange::AgentDefaults { mode, effort } => {
            written(config::set_agent_defaults(config, *mode, *effort, home))
        }
        SettingsChange::Restart => Ok(()),
    }
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RestartReply {
    /// `started`, or `alreadyRunning`: there was no `daemon.pid` to stop, so the old daemon still runs.
    pub outcome: &'static str,
    pub url: String,
    pub local: bool,
    pub we_started: bool,
}

/// egui's `restart_daemon`: stop, where a daemon without a pidfile is not an error, then connect or start.
pub fn restart_with(
    paths: &Paths,
    owner: &Owner,
    stop: impl FnOnce(&Connected) -> Result<(), StopError>,
    ensure: impl FnOnce(&ClientConfig) -> Result<Started, String>,
) -> Result<RestartReply, String> {
    let cfg = config::load(&paths.config, &paths.home).map_err(|e| e.to_string())?;
    // The daemon running is the one the window connected to; the config only says where the next one goes.
    let was = owner.connected().unwrap_or_else(|| Connected::of(&cfg));
    match stop(&was) {
        Ok(()) | Err(StopError::NoPidfile) => {}
        Err(e) => return Err(e.to_string()),
    }
    let started = ensure(&cfg)?;
    own(owner, &started, &cfg);
    if started == Started::AlreadyRunning {
        // Nothing was stopped: the daemon still runs with its folders, and the ones written since still wait
        // (egui applies pending folders only when it started the daemon).
        let mut state = owner.state();
        if let Some(c) = state.connected.as_mut() {
            c.tasks_root = was.tasks_root;
            c.personas_dir = was.personas_dir;
        }
    }
    let url = cfg.base_url();
    Ok(RestartReply {
        outcome: match started {
            Started::StartedByUs(_) => "started",
            Started::AlreadyRunning => "alreadyRunning",
        },
        local: is_loopback_url(&url),
        url,
        we_started: owner.pid().is_some(),
    })
}

/// The write, then the restart, in that order; a failed write never restarts.
pub fn save_then_restart(
    paths: &Paths,
    owner: &Owner,
    change: &SettingsChange,
    stop: impl FnOnce(&Connected) -> Result<(), StopError>,
    ensure: impl FnOnce(&ClientConfig) -> Result<Started, String>,
) -> Result<RestartReply, String> {
    apply_change(paths, change)?;
    restart_with(paths, owner, stop, ensure)
}

/// The relaunched wizard's folders: written now, used when the daemon next starts (egui `save_wizard_folders`).
pub fn save_settings_to(paths: &Paths, change: &SettingsChange) -> Result<(), String> {
    if *change == SettingsChange::Restart {
        return Err("Nothing to save".into());
    }
    apply_change(paths, change)
}

#[tauri::command]
pub fn save_settings(paths: tauri::State<'_, Paths>, change: SettingsChange) -> Result<(), String> {
    save_settings_to(&paths, &change)
}

/// Settings' Saves, Restart daemon, and the version offer's Restart. Blocks for the stop and the start's wait, so it
/// runs off the async runtime's workers.
#[tauri::command]
pub async fn save_and_restart(
    paths: tauri::State<'_, Paths>,
    owner: tauri::State<'_, Owner>,
    change: SettingsChange,
) -> Result<RestartReply, String> {
    let (paths, owner) = (paths.inner().clone(), owner.inner().clone());
    tauri::async_runtime::spawn_blocking(move || {
        save_then_restart(
            &paths,
            &owner,
            &change,
            |was| stop_daemon(&was.state_dir, &was.base_url),
            |cfg| connect_or_start(cfg, find_daemon_binary, START_WAIT).map_err(|e| e.to_string()),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The Daemon tab's rows.
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DaemonInfo {
    pub address: String,
    pub started_by: &'static str,
    pub state_dir: String,
    pub config_file: String,
    /// `daemon.pid` trimmed; None without the file or with a blank one.
    pub pid: Option<String>,
}

pub fn daemon_info_of(paths: &Paths, owner: &Owner) -> DaemonInfo {
    let Connected {
        listen, state_dir, ..
    } = owner
        .connected()
        .unwrap_or_else(|| Connected::of(&paths.load()));
    DaemonInfo {
        address: listen,
        started_by: if owner.pid().is_some() {
            "this window"
        } else {
            "already running"
        },
        state_dir: state_dir.display().to_string(),
        config_file: paths.config.display().to_string(),
        pid: std::fs::read_to_string(state_dir.join("daemon.pid"))
            .ok()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty()),
    }
}

/// Read each time, as egui reads the pidfile on every frame the tab is drawn.
#[tauri::command]
pub fn daemon_info(paths: tauri::State<'_, Paths>, owner: tauri::State<'_, Owner>) -> DaemonInfo {
    daemon_info_of(&paths, &owner)
}

/// What a check says, for the new-version notice and Settings' Version line. A skipped release is still available
/// to Settings, which shows it; only the notice leaves it out.
#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum UpdateCheck {
    Current,
    Failed,
    Available {
        version: String,
        notes: String,
        page: String,
        skipped: bool,
    },
}

pub fn update_check_for(
    current: &str,
    answer: Result<Release, String>,
    skipped: Option<&str>,
) -> UpdateCheck {
    match UpdateState::from_answer(current, &answer) {
        UpdateState::Available(r) => UpdateCheck::Available {
            skipped: skipped == Some(r.version.as_str()),
            version: r.version,
            notes: r.notes,
            page: r.page,
        },
        UpdateState::UpToDate => UpdateCheck::Current,
        UpdateState::Unknown | UpdateState::Failed => UpdateCheck::Failed,
    }
}

#[tauri::command]
pub async fn check_update(paths: tauri::State<'_, Paths>) -> Result<UpdateCheck, String> {
    let settings = paths.settings();
    tauri::async_runtime::spawn_blocking(move || {
        let skipped = SettingsWriter::shared().current(&settings).skipped_version;
        let answer = latest(&releases_url());
        if let Err(e) = &answer {
            tracing::warn!("Could not check for a new version: {e}");
        }
        update_check_for(env!("CARGO_PKG_VERSION"), answer, skipped.as_deref())
    })
    .await
    .map_err(|e| e.to_string())
}

/// "Skip this version": its notice does not show again; a later release's does.
#[tauri::command]
pub fn skip_version(paths: tauri::State<'_, Paths>, version: String) {
    SettingsWriter::shared().update(
        &paths.settings(),
        move |s| s.skipped_version = Some(version.clone()),
        |_| {},
    );
}

/// The board's choices as egui keeps them in `gui.toml`, so both apps share them.
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Choices {
    pub persona: Option<String>,
    pub product: Option<String>,
    pub display: String,
    /// The preferences Settings shows, under `gui.toml`'s own names (`dark`, `space_grotesk`, `keep_running`).
    pub theme: String,
    pub ui_font: String,
    pub mono_font: String,
    pub animations: bool,
    pub sticky_alerts: bool,
    pub close_choice: Option<String>,
}

const THEMES: [ThemeChoice; 3] = [ThemeChoice::System, ThemeChoice::Dark, ThemeChoice::Light];
const CLOSE_CHOICES: [CloseChoice; 2] = [CloseChoice::KeepRunning, CloseChoice::StopDaemon];

/// A setting's name in `gui.toml`, as serde writes it.
fn toml_name<T: Serialize>(v: T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(Into::into))
        .unwrap_or_default()
}

/// The one of `all` named `value`; anything else is refused, so a bad value fails the call instead of saving.
fn named<T: Serialize + Copy>(all: &[T], key: &str, value: Option<&str>) -> Result<T, String> {
    all.iter()
        .copied()
        .find(|v| Some(toml_name(*v).as_str()) == value)
        .ok_or_else(|| format!("no {key} {value:?}"))
}

fn flag(key: &str, value: Option<&str>) -> Result<bool, String> {
    match value {
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        _ => Err(format!("no {key} {value:?}")),
    }
}

fn display_name(d: FleetDisplay) -> &'static str {
    match d {
        FleetDisplay::Show => "show",
        FleetDisplay::Compact => "compact",
        FleetDisplay::Hide => "hide",
    }
}

pub fn choices_of(s: &GuiSettings) -> Choices {
    Choices {
        persona: s.active_persona.clone(),
        product: s.active_product.clone(),
        display: display_name(s.fleet_display).into(),
        theme: toml_name(s.theme),
        ui_font: toml_name(s.ui_font),
        mono_font: toml_name(s.mono_font),
        animations: s.animations,
        sticky_alerts: s.sticky_alerts,
        close_choice: s.close_choice.map(toml_name),
    }
}

/// One web choice (`web/src/shell/choices.ts`'s keys) into egui's settings.
pub fn apply_choice(s: &mut GuiSettings, key: &str, value: Option<&str>) -> Result<(), String> {
    match key {
        "fleet.persona" => s.active_persona = value.map(Into::into),
        "fleet.product" => s.active_product = value.map(Into::into),
        "fleet.display" => {
            s.fleet_display = FleetDisplay::ALL
                .into_iter()
                .find(|d| Some(display_name(*d)) == value)
                .ok_or_else(|| format!("no fleet display {value:?}"))?
        }
        "fleet.theme" => s.theme = named(&THEMES, key, value)?,
        "fleet.uiFont" => s.ui_font = named(&UiFont::ALL, key, value)?,
        "fleet.monoFont" => s.mono_font = named(&MonoFont::ALL, key, value)?,
        "fleet.animations" => s.animations = flag(key, value)?,
        "fleet.stickyAlerts" => s.sticky_alerts = flag(key, value)?,
        // None is Ask: the close dialog asks again.
        "fleet.closeChoice" => {
            s.close_choice = value
                .map(|v| named(&CLOSE_CHOICES, key, Some(v)))
                .transpose()?
        }
        _ => return Err(format!("no choice {key}")),
    }
    Ok(())
}

/// The close choice made in this run wins over the file's, as the close flow goes by it (`close::Flow::remembered`).
pub fn with_chosen(mut c: Choices, chosen: Option<Option<CloseChoice>>) -> Choices {
    if let Some(choice) = chosen {
        c.close_choice = choice.map(toml_name);
    }
    c
}

#[tauri::command]
pub fn read_choices(
    paths: tauri::State<'_, Paths>,
    close: tauri::State<'_, crate::close::CloseState>,
) -> Choices {
    with_chosen(
        choices_of(&SettingsWriter::shared().current(&paths.settings())),
        close.chosen(),
    )
}

/// Resolves once `gui.toml` is written, so Settings can say "Settings saved" or why not.
#[tauri::command]
pub async fn write_choice(
    paths: tauri::State<'_, Paths>,
    close: tauri::State<'_, crate::close::CloseState>,
    key: String,
    value: Option<String>,
) -> Result<(), String> {
    // Checked here, so a bad key fails the call instead of the background write.
    let mut checked = GuiSettings::default();
    apply_choice(&mut checked, &key, value.as_deref())?;
    if key == "fleet.closeChoice" {
        // The close flow goes by it at once, whether or not the save lands.
        close.choose(checked.close_choice);
    }
    let (tx, rx) = std::sync::mpsc::channel();
    SettingsWriter::shared().update(
        &paths.settings(),
        move |s| {
            let _ = apply_choice(s, &key, value.as_deref());
        },
        move |result| {
            let _ = tx.send(result.map(|_| ()).map_err(|e| e.to_string()));
        },
    );
    tauri::async_runtime::spawn_blocking(move || rx.recv().map_err(|e| e.to_string())?)
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn paths_in(dir: &Path, toml: Option<&str>) -> Paths {
        let config = dir.join("config.toml");
        if let Some(t) = toml {
            std::fs::write(&config, t).unwrap();
        }
        Paths {
            config,
            home: dir.to_path_buf(),
        }
    }

    #[test]
    fn a_running_daemon_on_port_0_is_reached_at_the_address_it_wrote() {
        let dir = tempfile::tempdir().unwrap();
        // Answers `is_up` as a daemon would.
        let daemon = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = daemon.local_addr().unwrap();
        std::thread::spawn(move || {
            for mut s in daemon.incoming().flatten() {
                let _ = std::io::Read::read(&mut s, &mut [0; 512]);
                let _ = std::io::Write::write_all(
                    &mut s,
                    b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n",
                );
            }
        });
        let state = dir.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(state.join("daemon.addr"), addr.to_string()).unwrap();
        let toml = format!(
            "listen = \"127.0.0.1:0\"\nstate_dir = \"{}\"\n",
            state.display()
        );
        let reply = start_with(
            &paths_in(dir.path(), Some(&toml)),
            &Owner::default(),
            || None,
        )
        .unwrap();
        assert_eq!(
            reply,
            StartReply {
                we_started: false,
                url: format!("http://{addr}"),
                local: true,
            }
        );
    }

    #[test]
    fn a_broken_config_stops_the_start_with_its_reason() {
        let dir = tempfile::tempdir().unwrap();
        let err = start_with(
            &paths_in(dir.path(), Some("listen = [")),
            &Owner::default(),
            || None,
        )
        .unwrap_err();
        assert!(err.contains("config.toml"), "{err}");
    }

    #[test]
    fn no_daemon_and_no_binary_says_so() {
        let dir = tempfile::tempdir().unwrap();
        // Port 1 never answers, so the start looks for a binary and finds none.
        let toml = format!(
            "listen = \"127.0.0.1:1\"\nstate_dir = \"{}\"\n",
            dir.path().join("state").display()
        );
        let owner = Owner::default();
        let err = start_with(&paths_in(dir.path(), Some(&toml)), &owner, || None).unwrap_err();
        assert_eq!(
            err,
            "fleet-daemon was not found next to fleet-gui or on PATH"
        );
        assert_eq!(
            owner.pid(),
            None,
            "nothing was started, so this window owns no daemon"
        );
    }

    #[test]
    fn a_broken_config_falls_back_to_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(dir.path(), Some("listen = ["));
        let defaults = config::defaults(&paths.config, &paths.home);
        assert_eq!(paths.load().state_dir, defaults.state_dir);
    }

    #[test]
    fn the_state_dir_is_the_configs() {
        let dir = tempfile::tempdir().unwrap();
        let toml = format!("state_dir = \"{}\"\n", dir.path().join("st").display());
        assert_eq!(
            paths_in(dir.path(), Some(&toml)).load().state_dir,
            dir.path().join("st")
        );
    }

    #[test]
    fn the_first_run_fields_are_egui_s() {
        let dir = tempfile::tempdir().unwrap();
        let d = first_run_defaults_of(&paths_in(dir.path(), None));
        assert_eq!(d.root, "~/fleet-tasks");
        assert!(
            d.personas.starts_with("~/") && d.personas.ends_with("personas"),
            "{}",
            d.personas
        );
    }

    #[test]
    fn the_first_run_writes_the_config_and_creates_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(dir.path(), None);
        save_first_run(&paths, "~/work/tasks", "~/ps").unwrap();
        assert!(dir.path().join("work/tasks").is_dir());
        let cfg = config::load(&paths.config, &paths.home).unwrap();
        assert_eq!(cfg.tasks_root, dir.path().join("work/tasks"));
    }

    #[test]
    fn an_empty_personas_folder_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(dir.path(), None);
        assert_eq!(
            save_first_run(&paths, "~/t", "  "),
            Err("Enter a folder for the personas".to_string())
        );
        assert!(!paths.config.exists());
    }

    #[test]
    fn a_config_written_meanwhile_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(dir.path(), Some("listen = \"127.0.0.1:9\"\n"));
        save_first_run(&paths, "~/t", "~/ps").unwrap();
        assert_eq!(
            std::fs::read_to_string(&paths.config).unwrap(),
            "listen = \"127.0.0.1:9\"\n"
        );
    }

    #[test]
    fn browse_opens_at_the_field_then_the_last_pick_then_home() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let settings = home.join("gui.toml");
        std::fs::create_dir_all(home.join("a/b")).unwrap();
        assert_eq!(pick_start(Some("~/a/b"), &settings, home), home.join("a/b"));
        assert_eq!(pick_start(Some("~/nope"), &settings, home), home);
        remember_pick(&settings, &home.join("a/b"));
        SettingsWriter::shared().flush();
        // egui's folder mode (`folder_mode_select_takes_the_current_folder`): the folder picked, which is the listing
        // Select was clicked in.
        assert_eq!(pick_start(None, &settings, home), home.join("a/b"));
        // A padded field still opens at its folder; a gone last pick falls back to home.
        assert_eq!(pick_start(Some("  ~/a  "), &settings, home), home.join("a"));
        std::fs::remove_dir_all(home.join("a")).unwrap();
        assert_eq!(pick_start(None, &settings, home), home);
    }

    /// A config in `dir` with its state dir there too, for the restart tests.
    fn daemon_paths(dir: &Path) -> Paths {
        let state = dir.join("state");
        std::fs::create_dir_all(&state).unwrap();
        paths_in(dir, Some(&format!("state_dir = \"{}\"\n", state.display())))
    }

    #[test]
    fn a_real_stopper_without_a_pidfile_goes_on_to_ensure() {
        let dir = tempfile::tempdir().unwrap();
        let paths = daemon_paths(dir.path());
        let reply = restart_with(
            &paths,
            &Owner::default(),
            |was| fleet_client::daemon_control::stop_daemon(&was.state_dir, &was.base_url),
            |_| Ok(Started::AlreadyRunning),
        )
        .unwrap();
        assert_eq!(reply.outcome, "alreadyRunning");
        assert!(!reply.we_started);
    }

    #[test]
    fn a_restart_makes_the_window_own_the_daemon() {
        let dir = tempfile::tempdir().unwrap();
        let owner = Owner::default();
        let reply = restart_with(
            &daemon_paths(dir.path()),
            &owner,
            |_| Ok(()),
            |_| Ok(Started::StartedByUs(4242)),
        )
        .unwrap();
        assert_eq!((reply.outcome, reply.we_started), ("started", true));
        assert_eq!(owner.pid(), Some(4242));
    }

    #[test]
    fn a_restart_that_fails_to_start_ends_with_its_reason() {
        let dir = tempfile::tempdir().unwrap();
        let err = restart_with(
            &daemon_paths(dir.path()),
            &Owner::default(),
            |_| Ok(()),
            |_| Err("fleet-daemon exited at startup".into()),
        )
        .unwrap_err();
        assert_eq!(err, "fleet-daemon exited at startup");
        // A stop that fails for another reason stops the restart there.
        let err = restart_with(
            &daemon_paths(dir.path()),
            &Owner::default(),
            |_| Err(StopError::Signal("denied".into())),
            |_| panic!("no start after a failed stop"),
        )
        .unwrap_err();
        assert_eq!(err, StopError::Signal("denied".into()).to_string());
    }

    #[test]
    fn a_restart_stops_the_daemon_the_window_connected_to() {
        // egui's stopper is the one it connected with: a state_dir edited by hand since must not hide the running
        // daemon's pidfile, or a second daemon starts beside it.
        let dir = tempfile::tempdir().unwrap();
        let paths = daemon_paths(dir.path());
        let owner = Owner::default();
        restart_with(&paths, &owner, |_| Ok(()), |_| Ok(Started::AlreadyRunning)).unwrap();
        let moved = dir.path().join("moved");
        std::fs::write(
            &paths.config,
            format!(
                "state_dir = \"{}\"\nlisten = \"127.0.0.1:9999\"\n",
                moved.display()
            ),
        )
        .unwrap();
        let info = daemon_info_of(&paths, &owner);
        assert_eq!(
            info.address, "127.0.0.1:7878",
            "the address the window talks to"
        );
        assert_eq!(
            info.state_dir,
            dir.path().join("state").display().to_string()
        );
        let mut stopped = None;
        let reply = restart_with(
            &paths,
            &owner,
            |was| {
                stopped = Some((was.state_dir.clone(), was.base_url.clone()));
                Ok(())
            },
            |_| Ok(Started::StartedByUs(7)),
        )
        .unwrap();
        assert_eq!(
            stopped,
            Some((
                dir.path().join("state"),
                "http://127.0.0.1:7878".to_string()
            ))
        );
        assert_eq!(
            reply.url, "http://127.0.0.1:9999",
            "the new daemon is where the config says"
        );
        assert_eq!(daemon_info_of(&paths, &owner).address, "127.0.0.1:9999");
    }

    #[test]
    fn running_folders_are_the_ones_connected_with() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        let toml = |root: &str| {
            format!(
                "state_dir = \"{}\"\ntasks_root = \"{root}\"\n",
                state.display()
            )
        };
        let paths = paths_in(dir.path(), Some(&toml("~/tasks")));
        let owner = Owner::default();
        restart_with(&paths, &owner, |_| Ok(()), |_| Ok(Started::AlreadyRunning)).unwrap();
        std::fs::write(&paths.config, toml("~/tasks2")).unwrap();
        let read = |owner: &Owner| {
            let cfg = config::load(&paths.config, &paths.home).unwrap();
            daemon_settings_of(&cfg, &paths.home, owner.connected().as_ref())
        };
        let before = read(&owner);
        assert_eq!(
            (
                before.tasks_root.as_str(),
                before.running_tasks_root.as_str()
            ),
            ("~/tasks2", "~/tasks")
        );
        restart_with(&paths, &owner, |_| Ok(()), |_| Ok(Started::StartedByUs(1))).unwrap();
        assert_eq!(read(&owner).running_tasks_root, "~/tasks2");
        // Before any connect the config is all there is.
        assert_eq!(read(&Owner::default()).running_tasks_root, "~/tasks2");
    }

    #[test]
    fn a_restart_that_finds_the_old_daemon_keeps_its_folders_running() {
        // No pidfile: nothing was stopped and the old daemon still runs with its folders, so the written ones wait
        // (egui applies pending folders only on StartedByUs).
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        let toml = |root: &str| format!("state_dir = \"{}\"\ntasks_root = \"{root}\"\n", state.display());
        let paths = paths_in(dir.path(), Some(&toml("~/tasks")));
        let owner = Owner::default();
        restart_with(&paths, &owner, |_| Ok(()), |_| Ok(Started::AlreadyRunning)).unwrap();
        std::fs::write(&paths.config, toml("~/tasks2")).unwrap();
        let reply = restart_with(
            &paths,
            &owner,
            |_| Err(StopError::NoPidfile),
            |_| Ok(Started::AlreadyRunning),
        )
        .unwrap();
        assert_eq!(reply.outcome, "alreadyRunning");
        let cfg = config::load(&paths.config, &paths.home).unwrap();
        let read = daemon_settings_of(&cfg, &paths.home, owner.connected().as_ref());
        assert_eq!(read.running_tasks_root, "~/tasks");
    }

    #[test]
    fn save_settings_writes_without_restarting() {
        let dir = tempfile::tempdir().unwrap();
        let paths = daemon_paths(dir.path());
        save_settings_to(
            &paths,
            &SettingsChange::TasksRoot {
                text: "~/t3".into(),
            },
        )
        .unwrap();
        assert!(std::fs::read_to_string(&paths.config)
            .unwrap()
            .contains("tasks_root = \"~/t3\""));
        assert_eq!(
            save_settings_to(&paths, &SettingsChange::Restart),
            Err("Nothing to save".into())
        );
    }

    #[test]
    fn a_failed_write_does_not_restart() {
        let dir = tempfile::tempdir().unwrap();
        // The config's folder is a file, so nothing can be written there.
        std::fs::write(dir.path().join("blocked"), "").unwrap();
        let paths = Paths {
            config: dir.path().join("blocked/config.toml"),
            home: dir.path().to_path_buf(),
        };
        let change = SettingsChange::Limits(LimitsDto {
            max_steps: 30,
            ..LimitsDto::from(config::Limits::default())
        });
        let err = save_then_restart(
            &paths,
            &Owner::default(),
            &change,
            |_| panic!("no stop"),
            |_| panic!("no start"),
        )
        .unwrap_err();
        assert!(
            err.starts_with(&format!("Cannot write {}: ", paths.config.display())),
            "{err}"
        );
    }

    #[test]
    fn each_change_writes_egui_s_keys() {
        let dir = tempfile::tempdir().unwrap();
        let paths = daemon_paths(dir.path());
        let home = dir.path();
        apply_change(
            &paths,
            &SettingsChange::TasksRoot {
                text: "~/tasks2".into(),
            },
        )
        .unwrap();
        assert!(home.join("tasks2").is_dir(), "the tasks folder is made");
        apply_change(
            &paths,
            &SettingsChange::PersonasDir {
                text: "~/personas2".into(),
            },
        )
        .unwrap();
        assert!(
            !home.join("personas2").exists(),
            "the personas folder is left for the daemon"
        );
        apply_change(
            &paths,
            &SettingsChange::Limits(LimitsDto {
                max_breakdown_versions: 4,
                max_steps: 30,
                max_parallel_specialists: 2,
                ..LimitsDto::from(config::Limits::default())
            }),
        )
        .unwrap();
        apply_change(
            &paths,
            &SettingsChange::Pr(PrDto {
                style: "conventional".into(),
                attribution: String::new(),
                draft: true,
                labels: vec!["fleet".into(), "bot".into()],
                gate_build: "clean".into(),
            }),
        )
        .unwrap();
        apply_change(
            &paths,
            &SettingsChange::AgentDefaults {
                mode: PermissionMode::Auto,
                effort: Effort::High,
            },
        )
        .unwrap();
        let text = std::fs::read_to_string(&paths.config).unwrap();
        for line in [
            "tasks_root = \"~/tasks2\"",
            "personas_dir = \"~/personas2\"",
            "max_steps = 30",
            "pr_style = \"conventional\"",
            "pr_draft = true",
            "pr_gate_build = \"clean\"",
            "default_effort = \"high\"",
        ] {
            assert!(text.contains(line), "{line} in\n{text}");
        }
        assert!(!text.contains("pr_attribution"), "{text}");
        assert!(!text.contains("default_mode"), "{text}");
        assert!(!text.contains("max_concurrent_tasks"), "{text}");
        let read = daemon_settings_of(&config::load(&paths.config, home).unwrap(), home, None);
        assert_eq!(read.tasks_root, "~/tasks2");
        assert_eq!(read.personas_dir, "~/personas2");
        assert_eq!(read.limits.max_steps, 30);
        assert_eq!(read.limits.max_concurrent_tasks, None);
        assert_eq!(read.pr.labels, ["fleet", "bot"]);
        assert_eq!(read.default_effort, Effort::High);
        assert_eq!(read.home, home.display().to_string());
    }

    #[test]
    fn an_empty_or_relative_folder_is_refused_with_egui_s_words() {
        let dir = tempfile::tempdir().unwrap();
        let paths = daemon_paths(dir.path());
        let before = std::fs::read_to_string(&paths.config).unwrap();
        assert_eq!(
            apply_change(&paths, &SettingsChange::PersonasDir { text: "  ".into() }),
            Err("Enter a folder for the personas".into())
        );
        assert_eq!(
            apply_change(&paths, &SettingsChange::TasksRoot { text: "".into() }),
            Err("Enter a folder for the tasks".into())
        );
        assert!(apply_change(
            &paths,
            &SettingsChange::TasksRoot {
                text: "rel/x".into()
            }
        )
        .unwrap_err()
        .ends_with("is not an absolute path"));
        assert_eq!(std::fs::read_to_string(&paths.config).unwrap(), before);
    }

    #[test]
    fn an_unknown_gate_build_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let pr = PrDto {
            style: "plain".into(),
            attribution: String::new(),
            draft: false,
            labels: vec![],
            gate_build: "dirty".into(),
        };
        assert!(apply_change(&daemon_paths(dir.path()), &SettingsChange::Pr(pr)).is_err());
    }

    #[test]
    fn the_page_s_changes_arrive_as_sent() {
        // The shapes web/src/native/native.ts's SettingsChange sends.
        let parse = |v: serde_json::Value| serde_json::from_value::<SettingsChange>(v).unwrap();
        assert_eq!(
            parse(serde_json::json!({"kind": "tasksRoot", "text": "~/t"})),
            SettingsChange::TasksRoot { text: "~/t".into() }
        );
        assert_eq!(
            parse(serde_json::json!({"kind": "restart"})),
            SettingsChange::Restart
        );
        let limits = parse(serde_json::json!({
            "kind": "limits", "maxBreakdownVersions": 8, "maxSteps": 30, "maxParallelSpecialists": 3,
            "maxPrAttempts": 3, "maxConcurrentTasks": null, "maxActiveProducts": 2
        }));
        assert!(
            matches!(limits, SettingsChange::Limits(l) if l.max_steps == 30 && l.max_active_products == Some(2))
        );
        let pr = parse(serde_json::json!({
            "kind": "pr", "style": "plain", "attribution": "", "draft": true, "labels": ["a"], "gateBuild": "clean"
        }));
        assert!(matches!(pr, SettingsChange::Pr(p) if p.draft && p.gate_build == "clean"));
        assert_eq!(
            parse(
                serde_json::json!({"kind": "agentDefaults", "mode": "acceptEdits", "effort": "xhigh"})
            ),
            SettingsChange::AgentDefaults {
                mode: PermissionMode::AcceptEdits,
                effort: Effort::Xhigh
            }
        );
    }

    #[test]
    fn daemon_info_reads_the_pidfile_each_time() {
        let dir = tempfile::tempdir().unwrap();
        let paths = daemon_paths(dir.path());
        let state = dir.path().join("state");
        let info = daemon_info_of(&paths, &Owner::default());
        assert_eq!(info.pid, None);
        assert_eq!(info.started_by, "already running");
        assert_eq!(info.config_file, paths.config.display().to_string());
        assert_eq!(info.state_dir, state.display().to_string());
        assert_eq!(info.address, "127.0.0.1:7878");
        std::fs::write(state.join("daemon.pid"), "4242\n").unwrap();
        let owner = Owner::default();
        own(
            &owner,
            &Started::StartedByUs(4242),
            &config::load(&paths.config, &paths.home).unwrap(),
        );
        let info = daemon_info_of(&paths, &owner);
        assert_eq!(
            (info.pid.as_deref(), info.started_by),
            (Some("4242"), "this window")
        );
        std::fs::write(state.join("daemon.pid"), "  ").unwrap();
        assert_eq!(
            daemon_info_of(&paths, &owner).pid,
            None,
            "a blank pidfile shows no PID"
        );
    }

    fn listed(personas: Vec<fleet_api::Persona>) -> PersonaList {
        serde_json::from_value(serde_json::json!({
            "personas": personas, "claude_command": ["claude", "--x"], "gh_command": "gh",
            "tasks_root": "/t", "claude_path": "/usr/bin/claude", "gh_path": null
        }))
        .unwrap()
    }

    fn persona(id: &str) -> fleet_api::Persona {
        serde_json::from_value(serde_json::json!({
            "id": id, "name": id, "host": "github.com", "commit_name": "c", "commit_email": "e",
            "github_auth": "gh", "claude_dir": "/p/claude", "gh_dir": "/p/gh", "ready": false,
            "claude": null, "github": null, "tasks": 0, "nicknames": {}
        }))
        .unwrap()
    }

    #[test]
    fn a_login_runs_fleet_client_s_script_for_the_listed_persona() {
        let list = listed(vec![persona("work")]);
        let p = &list.personas[0];
        assert_eq!(
            login_script(&list, "work", Login::Claude),
            Ok(login::claude_login_script(
                p,
                &["/usr/bin/claude".into(), "--x".into()],
                "/t"
            ))
        );
        assert_eq!(
            login_script(&list, "work", Login::Gh),
            Ok(login::gh_login_script(p, "gh"))
        );
        assert_eq!(
            login_script(&list, "gone", Login::Gh),
            Err("The persona is gone".to_string())
        );
    }

    #[test]
    fn an_install_runs_only_the_daemon_s_fix_for_that_tool() {
        let list: ToolList = serde_json::from_value(serde_json::json!({
            "tools": [
                { "tool": "git", "required": true, "version": "2.4", "error": null },
                { "tool": "gh", "required": true, "version": null, "error": {
                    "kind": "not-installed", "tool": "gh",
                    "fix": { "install": "sudo dnf install gh", "link": "https://x.invalid", "manager": "dnf" } } }
            ],
            "checked_at": 0, "path": null
        }))
        .unwrap();
        assert_eq!(
            install_command(&list, ToolId::Gh),
            Ok("sudo dnf install gh".to_string())
        );
        assert_eq!(
            install_command(&list, ToolId::Git),
            Err("Git needs no install".to_string())
        );
    }

    #[test]
    fn choices_go_to_gui_toml_under_egui_s_names() {
        let mut s = GuiSettings::default();
        apply_choice(&mut s, "fleet.persona", Some("work")).unwrap();
        apply_choice(&mut s, "fleet.product", Some("pie")).unwrap();
        apply_choice(&mut s, "fleet.display", Some("hide")).unwrap();
        let c = choices_of(&s);
        assert_eq!(
            (
                c.persona.as_deref(),
                c.product.as_deref(),
                c.display.as_str()
            ),
            (Some("work"), Some("pie"), "hide")
        );
        apply_choice(&mut s, "fleet.persona", None).unwrap();
        assert_eq!(s.active_persona, None);
        assert!(apply_choice(&mut s, "fleet.display", Some("huge")).is_err());
        assert!(apply_choice(&mut s, "theme", Some("dark")).is_err());
    }

    #[test]
    fn each_preference_round_trips_through_its_key() {
        let mut s = GuiSettings::default();
        let defaults = choices_of(&s);
        assert_eq!(
            (
                defaults.theme.as_str(),
                defaults.ui_font.as_str(),
                defaults.mono_font.as_str(),
                defaults.animations,
                defaults.sticky_alerts,
                defaults.close_choice
            ),
            ("system", "plex", "plex", true, true, None)
        );
        for (key, value) in [
            ("fleet.theme", "dark"),
            ("fleet.uiFont", "space_grotesk"),
            ("fleet.monoFont", "jetbrains"),
            ("fleet.animations", "false"),
            ("fleet.stickyAlerts", "false"),
            ("fleet.closeChoice", "stop_daemon"),
        ] {
            apply_choice(&mut s, key, Some(value)).unwrap();
        }
        let c = choices_of(&s);
        assert_eq!(
            (
                c.theme.as_str(),
                c.ui_font.as_str(),
                c.mono_font.as_str(),
                c.animations,
                c.sticky_alerts,
                c.close_choice.as_deref()
            ),
            (
                "dark",
                "space_grotesk",
                "jetbrains",
                false,
                false,
                Some("stop_daemon")
            )
        );
        // What egui reads back from the file.
        assert_eq!(s.theme, config::ThemeChoice::Dark);
        assert_eq!(s.close_choice, Some(config::CloseChoice::StopDaemon));
    }

    #[test]
    fn this_run_s_close_choice_is_what_settings_reads_back() {
        let saved = GuiSettings {
            close_choice: Some(CloseChoice::StopDaemon),
            ..GuiSettings::default()
        };
        // Settings chose Keep, and the save failed: Settings shows Keep, not the file's Stop.
        let c = with_chosen(choices_of(&saved), Some(Some(CloseChoice::KeepRunning)));
        assert_eq!(c.close_choice.as_deref(), Some("keep_running"));
        assert_eq!(
            with_chosen(choices_of(&saved), Some(None)).close_choice,
            None
        );
        assert_eq!(
            with_chosen(choices_of(&saved), None)
                .close_choice
                .as_deref(),
            Some("stop_daemon")
        );
    }

    #[test]
    fn ask_clears_the_close_choice() {
        let mut s = GuiSettings::default();
        apply_choice(&mut s, "fleet.closeChoice", Some("keep_running")).unwrap();
        apply_choice(&mut s, "fleet.closeChoice", None).unwrap();
        assert_eq!(s.close_choice, None);
    }

    #[test]
    fn an_unknown_preference_value_is_refused() {
        let mut s = GuiSettings::default();
        assert!(apply_choice(&mut s, "fleet.theme", Some("sepia")).is_err());
        assert!(apply_choice(&mut s, "fleet.uiFont", Some("comic")).is_err());
        assert!(apply_choice(&mut s, "fleet.animations", Some("maybe")).is_err());
        assert!(apply_choice(&mut s, "fleet.theme", None).is_err());
    }

    #[test]
    fn a_skipped_release_is_still_available_to_settings() {
        let release = |v: &str| Release {
            version: v.into(),
            notes: "n".into(),
            page: "https://x.invalid".into(),
        };
        let available = |v: &str, skipped| UpdateCheck::Available {
            version: v.into(),
            notes: "n".into(),
            page: "https://x.invalid".into(),
            skipped,
        };
        assert_eq!(
            update_check_for("0.2.0", Ok(release("0.3.0")), None),
            available("0.3.0", false)
        );
        assert_eq!(
            update_check_for("0.2.0", Ok(release("0.3.0")), Some("0.3.0")),
            available("0.3.0", true)
        );
        assert_eq!(
            update_check_for("0.2.0", Ok(release("0.4.0")), Some("0.3.0")),
            available("0.4.0", false)
        );
        assert_eq!(
            update_check_for("0.2.0", Ok(release("0.2.0")), None),
            UpdateCheck::Current
        );
        assert_eq!(
            update_check_for("0.2.0", Err("offline".into()), None),
            UpdateCheck::Failed
        );
    }
}
