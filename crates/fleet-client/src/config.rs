//! What the client needs from the daemon's config file (`config.toml` in `fleet_fs::dirs::config_dir`, so every key the
//! client does not use is ignored) and its own small settings file, `gui.toml`.
use fleet_api::{Effort, PermissionMode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex, OnceLock};

#[derive(Debug)]
pub enum ConfigError {
    Io(PathBuf, std::io::Error),
    Parse(PathBuf, toml::de::Error),
    /// The daemon refuses to bind it, so whatever answers there is not ours.
    NonLoopback(PathBuf, SocketAddr),
    /// A key the daemon no longer reads; the file is the user's, so it is named, not rewritten.
    RenamedKey {
        old: &'static str,
        new: &'static str,
        path: PathBuf,
    },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(p, e) => write!(f, "cannot read config {}: {e}", p.display()),
            ConfigError::Parse(p, e) => write!(f, "invalid config {}: {e}", p.display()),
            ConfigError::RenamedKey { old, new, path } => {
                write!(
                    f,
                    "{old} was renamed to {new}; rename it in {}",
                    path.display()
                )
            }
            ConfigError::NonLoopback(p, a) => {
                write!(f, "listen address {a} in {} is not loopback; the daemon only binds 127.0.0.1 or ::1", p.display())
            }
        }
    }
}

impl std::error::Error for ConfigError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientConfig {
    pub listen: SocketAddr,
    pub state_dir: PathBuf,
    pub tasks_root: PathBuf,
    pub personas_dir: PathBuf,
    pub limits: Limits,
    pub pr: PrSettings,
    /// The daemon's global defaults for every agent's `--permission-mode` and `--effort`. A value the daemon would
    /// refuse reads as the default here; the daemon names the problem when it starts.
    pub default_mode: PermissionMode,
    pub default_effort: Effort,
    /// The file this was read from; handed to a daemon the client starts.
    pub path: PathBuf,
}

/// The daemon's four global limits (`max_*` keys). The daemon reads them once at startup, so a change applies
/// after it restarts. The defaults are the daemon's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_breakdown_versions: u32,
    pub max_steps: u32,
    pub max_parallel_specialists: u32,
    pub max_pr_attempts: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_breakdown_versions: 8,
            max_steps: 20,
            max_parallel_specialists: 3,
            max_pr_attempts: 3,
        }
    }
}

/// The daemon's global pull request settings (`pr_*` keys). The daemon reads them once at startup, so a change
/// applies after it restarts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrSettings {
    /// Name of a `pr_style` definition.
    pub style: String,
    pub attribution: String,
    pub draft: bool,
    pub labels: Vec<String>,
}

impl Default for PrSettings {
    fn default() -> Self {
        PrSettings {
            style: "plain".into(),
            attribution: String::new(),
            draft: false,
            labels: vec![],
        }
    }
}

impl ClientConfig {
    /// With `listen` on port 0 the daemon picks its port, so the address is the one it wrote to `daemon.addr`. Until
    /// there is one, the configured address stands in, and nothing answers on port 0.
    pub fn base_url(&self) -> String {
        let addr = if self.listen.port() == 0 {
            self.bound_addr().unwrap_or(self.listen)
        } else {
            self.listen
        };
        format!("http://{addr}")
    }

    fn bound_addr(&self) -> Option<SocketAddr> {
        let addr: SocketAddr = std::fs::read_to_string(self.addrfile())
            .ok()?
            .trim()
            .parse()
            .ok()?;
        (addr.ip().is_loopback() && addr.port() != 0).then_some(addr)
    }

    pub fn addrfile(&self) -> PathBuf {
        self.state_dir.join("daemon.addr")
    }

    pub fn pidfile(&self) -> PathBuf {
        self.state_dir.join("daemon.pid")
    }

    pub fn log_file(&self) -> PathBuf {
        self.state_dir.join("daemon.log")
    }
}

#[derive(Deserialize, Default)]
struct Raw {
    listen: Option<SocketAddr>,
    state_dir: Option<String>,
    tasks_root: Option<String>,
    /// Only to refuse it with a pointer to the new name.
    projects_root: Option<String>,
    personas_dir: Option<String>,
    max_breakdown_versions: Option<u32>,
    max_steps: Option<u32>,
    max_parallel_specialists: Option<u32>,
    max_pr_attempts: Option<u32>,
    default_mode: Option<String>,
    default_effort: Option<String>,
    pr_style: Option<String>,
    pr_attribution: Option<String>,
    pr_draft: Option<bool>,
    pr_labels: Option<Vec<String>>,
}

/// The path given as `--config PATH` or `--config=PATH` among `args` (the program name excluded).
pub fn config_arg(args: impl IntoIterator<Item = String>) -> Option<PathBuf> {
    let mut args = args.into_iter();
    while let Some(a) = args.next() {
        if let Some(p) = a.strip_prefix("--config=") {
            return Some(PathBuf::from(p));
        }
        if a == "--config" {
            return args.next().map(PathBuf::from);
        }
    }
    None
}

pub fn default_path(home: &Path) -> PathBuf {
    fleet_fs::dirs::config_dir(home).join("config.toml")
}

pub fn gui_settings_path(home: &Path) -> PathBuf {
    fleet_fs::dirs::config_dir(home).join("gui.toml")
}

pub fn exists(path: &Path) -> bool {
    path.is_file()
}

/// `~` and `~/…` become paths under `home`; anything else is taken as written.
pub fn expand_home(p: &str, home: &Path) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if p == "~" => home.to_path_buf(),
        None => PathBuf::from(p),
    }
}

/// A missing file is the defaults, the same as in the daemon, so the app opens before any setup.
pub fn load(path: &Path, home: &Path) -> Result<ClientConfig, ConfigError> {
    let raw: Raw = match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).map_err(|e| ConfigError::Parse(path.to_path_buf(), e))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Raw::default(),
        Err(e) => return Err(ConfigError::Io(path.to_path_buf(), e)),
    };
    if raw.projects_root.is_some() {
        return Err(ConfigError::RenamedKey {
            old: "projects_root",
            new: "tasks_root",
            path: path.to_path_buf(),
        });
    }
    let cfg = from_raw(raw, path, home);
    if !cfg.listen.ip().is_loopback() {
        return Err(ConfigError::NonLoopback(path.to_path_buf(), cfg.listen));
    }
    Ok(cfg)
}

/// What `load` gives for a missing file at `path`.
pub fn defaults(path: &Path, home: &Path) -> ClientConfig {
    from_raw(Raw::default(), path, home)
}

fn from_raw(raw: Raw, path: &Path, home: &Path) -> ClientConfig {
    let state_dir = match raw.state_dir.as_deref() {
        Some(s) => expand_home(s, home),
        None => fleet_fs::dirs::data_dir(home),
    };
    let tasks_root = match raw.tasks_root.as_deref() {
        Some(s) => expand_home(s, home),
        None => home.join("fleet-tasks"),
    };
    let personas_dir = match raw.personas_dir.as_deref() {
        Some(s) => expand_home(s, home),
        None => default_personas_dir(home),
    };
    ClientConfig {
        listen: raw
            .listen
            .unwrap_or_else(|| "127.0.0.1:7878".parse().expect("static address")),
        state_dir,
        tasks_root,
        personas_dir,
        limits: Limits {
            max_breakdown_versions: raw
                .max_breakdown_versions
                .unwrap_or(Limits::default().max_breakdown_versions),
            max_steps: raw.max_steps.unwrap_or(Limits::default().max_steps),
            max_parallel_specialists: raw
                .max_parallel_specialists
                .unwrap_or(Limits::default().max_parallel_specialists),
            max_pr_attempts: raw
                .max_pr_attempts
                .unwrap_or(Limits::default().max_pr_attempts),
        },
        pr: {
            let default = PrSettings::default();
            PrSettings {
                style: raw.pr_style.unwrap_or(default.style),
                attribution: raw.pr_attribution.unwrap_or(default.attribution),
                draft: raw.pr_draft.unwrap_or(default.draft),
                labels: raw.pr_labels.unwrap_or(default.labels),
            }
        },
        default_mode: raw
            .default_mode
            .and_then(|m| m.parse().ok())
            .unwrap_or(PermissionMode::Auto),
        default_effort: raw
            .default_effort
            .and_then(|e| e.parse().ok())
            .unwrap_or(Effort::Medium),
        path: path.to_path_buf(),
    }
}

/// A path under `home` written with `~`, the way the config file and the user write it.
pub fn shorten_home(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Where the daemon keeps persona folders unless `personas_dir` says otherwise.
pub fn default_personas_dir(home: &Path) -> PathBuf {
    fleet_fs::dirs::data_dir(home).join("personas")
}

/// First-run file: the chosen folders on top, then the commented template so every other key stays discoverable.
pub fn write_initial(
    path: &Path,
    tasks_root: &Path,
    personas_dir: &Path,
    home: &Path,
) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let shown = shorten_home(tasks_root, home);
    let personas = shorten_home(personas_dir, home);
    // Debug formatting gives a valid TOML basic string for any path with quotes or backslashes.
    let body = format!(
        "tasks_root = {shown:?}\npersonas_dir = {personas:?}\n\n{}",
        include_str!("../../../packaging/config.example.toml")
    );
    // create_new so a hand-edited config is never overwritten, and two first runs cannot race.
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    f.write_all(body.as_bytes())
}

/// The tasks folder the user typed, ready to be written: trimmed, `~` expanded, absolute and existing. Any
/// failure is a message for the user.
pub fn prepare_tasks_root(typed: &str, home: &Path) -> Result<PathBuf, String> {
    let typed = typed.trim();
    if typed.is_empty() {
        return Err("Enter a folder for the tasks".into());
    }
    let dir = expand_home(typed, home);
    if !dir.is_absolute() {
        return Err(format!("{} is not an absolute path", dir.display()));
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    Ok(dir)
}

/// The personas folder the user typed, ready to be written: trimmed, `~` expanded and absolute. Not created: the
/// daemon makes it with the modes it needs. Any failure is a message for the user.
pub fn prepare_personas_dir(typed: &str, home: &Path) -> Result<PathBuf, String> {
    let typed = typed.trim();
    if typed.is_empty() {
        return Err("Enter a folder for the personas".into());
    }
    let dir = expand_home(typed, home);
    if !dir.is_absolute() {
        return Err(format!("{} is not an absolute path", dir.display()));
    }
    Ok(dir)
}

/// Points the daemon's config at `root`, changing only `tasks_root`: the file is the user's, with their
/// comments and every key the client does not know. A missing file gets the first-run file.
pub fn set_tasks_root(config: &Path, root: &Path, home: &Path) -> std::io::Result<()> {
    set_path(config, "tasks_root", root, home, || {
        write_initial(config, root, &default_personas_dir(home), home)
    })
}

/// Points the daemon's config at `dir`, changing only `personas_dir`, as `set_tasks_root` does. Persona folders
/// already made stay where they are.
pub fn set_personas_dir(config: &Path, dir: &Path, home: &Path) -> std::io::Result<()> {
    set_path(config, "personas_dir", dir, home, || {
        write_initial(config, &defaults(config, home).tasks_root, dir, home)
    })
}

/// Writes the limits that differ from what the file holds, as `set_tasks_root` does. A key the file lacks counts as
/// its default, so saving an untouched default does not pin it and later default changes still reach the user. A
/// missing file gets the first-run file first.
pub fn set_limits(config: &Path, limits: &Limits, home: &Path) -> std::io::Result<()> {
    if !exists(config) {
        let d = defaults(config, home);
        write_initial(config, &d.tasks_root, &d.personas_dir, home)?;
    }
    edit_config(
        config,
        || Err(std::io::ErrorKind::NotFound.into()),
        |doc| {
            let default = Limits::default();
            for (key, n, fallback) in [
                (
                    "max_breakdown_versions",
                    limits.max_breakdown_versions,
                    default.max_breakdown_versions,
                ),
                ("max_steps", limits.max_steps, default.max_steps),
                (
                    "max_parallel_specialists",
                    limits.max_parallel_specialists,
                    default.max_parallel_specialists,
                ),
                (
                    "max_pr_attempts",
                    limits.max_pr_attempts,
                    default.max_pr_attempts,
                ),
            ] {
                let held = doc
                    .get(key)
                    .and_then(toml_edit::Item::as_integer)
                    .and_then(|v| u32::try_from(v).ok())
                    .unwrap_or(fallback);
                if held != n {
                    set_value(doc, key, i64::from(n).into());
                }
            }
        },
    )
}

/// Writes the pull request settings that differ from what the file holds, as `set_limits` does: a key the file
/// lacks counts as its default and is not pinned.
pub fn set_pr(config: &Path, pr: &PrSettings, home: &Path) -> std::io::Result<()> {
    if !exists(config) {
        let d = defaults(config, home);
        write_initial(config, &d.tasks_root, &d.personas_dir, home)?;
    }
    edit_config(
        config,
        || Err(std::io::ErrorKind::NotFound.into()),
        |doc| {
            let held = from_raw(
                toml::from_str(&doc.to_string()).unwrap_or_default(),
                config,
                home,
            )
            .pr;
            if held.style != pr.style {
                set_value(doc, "pr_style", pr.style.as_str().into());
            }
            if held.attribution != pr.attribution {
                set_value(doc, "pr_attribution", pr.attribution.as_str().into());
            }
            if held.draft != pr.draft {
                set_value(doc, "pr_draft", pr.draft.into());
            }
            if held.labels != pr.labels {
                set_value(
                    doc,
                    "pr_labels",
                    pr.labels
                        .iter()
                        .map(String::as_str)
                        .collect::<toml_edit::Array>()
                        .into(),
                );
            }
        },
    )
}

/// Writes the default mode and effort that differ from what the file holds, as `set_pr` does: a key the file lacks
/// counts as its default and is not pinned.
pub fn set_agent_defaults(
    config: &Path,
    mode: PermissionMode,
    effort: Effort,
    home: &Path,
) -> std::io::Result<()> {
    if !exists(config) {
        let d = defaults(config, home);
        write_initial(config, &d.tasks_root, &d.personas_dir, home)?;
    }
    edit_config(
        config,
        || Err(std::io::ErrorKind::NotFound.into()),
        |doc| {
            let held = from_raw(
                toml::from_str(&doc.to_string()).unwrap_or_default(),
                config,
                home,
            );
            if held.default_mode != mode {
                set_value(doc, "default_mode", mode.as_flag().into());
            }
            if held.default_effort != effort {
                set_value(doc, "default_effort", effort.as_flag().into());
            }
        },
    )
}

/// The labels as typed, comma-separated: each trimmed, the empty ones dropped.
pub fn parse_labels(typed: &str) -> Vec<String> {
    typed
        .split(',')
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

/// A limit as typed: a positive whole number, which is what the daemon accepts.
pub fn parse_limit(typed: &str) -> Option<u32> {
    typed.trim().parse::<u32>().ok().filter(|n| *n > 0)
}

/// Sets the top-level `key` to `path`, written with `~`, and leaves the rest of the file as the user wrote it.
/// `initial` writes the file when there is none.
fn set_path(
    config: &Path,
    key: &str,
    path: &Path,
    home: &Path,
    initial: impl FnOnce() -> std::io::Result<()>,
) -> std::io::Result<()> {
    edit_config(config, initial, |doc| {
        set_value(doc, key, shorten_home(path, home).into())
    })
}

/// Sets the top-level `key` to `value`. Swapping the value alone keeps the comment after it on the same line.
fn set_value(doc: &mut toml_edit::DocumentMut, key: &str, value: toml_edit::Value) {
    match doc.get_mut(key).and_then(toml_edit::Item::as_value_mut) {
        Some(old) => {
            let decor = old.decor().clone();
            *old = value;
            *old.decor_mut() = decor;
        }
        None => {
            doc.insert(key, toml_edit::value(value));
        }
    }
}

/// Applies `edit` to the config and writes it back, leaving the rest of the file as the user wrote it.
/// `initial` writes the file when there is none.
fn edit_config(
    config: &Path,
    initial: impl FnOnce() -> std::io::Result<()>,
    edit: impl FnOnce(&mut toml_edit::DocumentMut),
) -> std::io::Result<()> {
    let text = match std::fs::read_to_string(config) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return initial(),
        Err(e) => return Err(e),
    };
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid config {}: {e}", config.display()),
        )
    })?;
    edit(&mut doc);
    // Through a temp file in the same directory and a rename, so a crash or a full disk never leaves the user's
    // hand-edited config cut short. On the resolved target, so a config symlinked from a dotfiles repo stays a
    // link and the file it points to gets the change.
    let target = fleet_fs::canonical(config)?;
    let mut tmp = target.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    // The user's mode (say 0600, for a config holding a token) must survive. The temp file is created with it, so
    // the text is never readable more widely, even for a moment; the umask can only narrow that, and the
    // set_permissions after the write puts back exactly the original.
    let perms = std::fs::metadata(&target)?.permissions();
    write_new(&tmp, doc.to_string().as_bytes(), &perms)
        .and_then(|()| std::fs::set_permissions(&tmp, perms))
        .and_then(|()| std::fs::rename(&tmp, &target))
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
}

/// Writes `bytes` to a new file at `path`, created with `perms` where the platform allows. A leftover file of that
/// name is removed first: opening it would keep its own, maybe wider, mode.
fn write_new(path: &Path, bytes: &[u8], perms: &std::fs::Permissions) -> std::io::Result<()> {
    use std::io::Write;
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        options.mode(perms.mode() & 0o7777);
    }
    #[cfg(not(unix))]
    let _ = perms;
    options.open(path)?.write_all(bytes)
}

/// What closing the window does to the daemon, when the user has said so once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseChoice {
    KeepRunning,
    StopDaemon,
}

/// Which look the window has; `System` follows the desktop's light/dark setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeChoice {
    Light,
    Dark,
    /// Also what an unknown value reads as, so a mistyped theme cannot cost the saved close choice. Last because
    /// serde only takes `other` on the last variant.
    #[default]
    #[serde(other)]
    System,
}

impl ThemeChoice {
    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::System => "System",
            ThemeChoice::Light => "Light",
            ThemeChoice::Dark => "Dark",
        }
    }
}

/// The face for everything but code; each is a regular and a medium file bundled with the GUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiFont {
    SpaceGrotesk,
    Atkinson,
    #[default]
    #[serde(other)]
    Plex,
}

impl UiFont {
    pub const ALL: [UiFont; 3] = [UiFont::Plex, UiFont::SpaceGrotesk, UiFont::Atkinson];

    pub fn label(self) -> &'static str {
        match self {
            UiFont::Plex => "IBM Plex Sans",
            UiFont::SpaceGrotesk => "Space Grotesk",
            UiFont::Atkinson => "Atkinson Hyperlegible Next",
        }
    }
}

/// The face for code, terminal output and labels; chosen apart from `UiFont` so any pair works.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonoFont {
    #[serde(rename = "jetbrains")]
    JetBrains,
    Atkinson,
    #[default]
    #[serde(other)]
    Plex,
}

impl MonoFont {
    pub const ALL: [MonoFont; 3] = [MonoFont::Plex, MonoFont::JetBrains, MonoFont::Atkinson];

    pub fn label(self) -> &'static str {
        match self {
            MonoFont::Plex => "IBM Plex Mono",
            MonoFont::JetBrains => "JetBrains Mono",
            MonoFont::Atkinson => "Atkinson Hyperlegible Mono",
        }
    }
}

/// Any value that is not one of the names, even a number from a hand edit, reads as the default; the rest of
/// the file is kept.
fn lenient<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    let raw = toml::Value::deserialize(d)?;
    Ok(T::deserialize(raw).unwrap_or_default())
}

/// How the board draws an orchestrated task's fleet: full cards, one-row entries, or only the task card's tally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FleetDisplay {
    Show,
    #[default]
    Compact,
    Hide,
}

impl FleetDisplay {
    pub const ALL: [FleetDisplay; 3] = [
        FleetDisplay::Show,
        FleetDisplay::Compact,
        FleetDisplay::Hide,
    ];

    pub fn label(self) -> &'static str {
        match self {
            FleetDisplay::Show => "Show",
            FleetDisplay::Compact => "Compact",
            FleetDisplay::Hide => "Hide",
        }
    }
}

/// An unknown value reads as the default, so a mistyped one cannot cost the rest of the file (`load` would fall back
/// to every default).
fn lenient_fleet_display<'de, D: serde::Deserializer<'de>>(d: D) -> Result<FleetDisplay, D::Error> {
    let raw = String::deserialize(d)?;
    Ok(
        FleetDisplay::deserialize(serde::de::value::StrDeserializer::<D::Error>::new(&raw))
            .unwrap_or_default(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuiSettings {
    #[serde(default)]
    pub close_choice: Option<CloseChoice>,
    #[serde(default)]
    pub theme: ThemeChoice,
    /// The folder the folder picker last selected from; it opens there next time.
    #[serde(default)]
    pub last_pick_dir: Option<PathBuf>,
    /// Errors and warnings stay in the footer, cycling with the other messages, until clicked away.
    #[serde(default = "yes")]
    pub sticky_alerts: bool,
    /// The persona the GUI last had selected; a name the daemon no longer lists is ignored by the reader.
    #[serde(default)]
    pub active_persona: Option<String>,
    /// The product the GUI last had selected; one the daemon no longer lists is ignored by the reader.
    #[serde(default)]
    pub active_product: Option<String>,
    #[serde(default, deserialize_with = "lenient_fleet_display")]
    pub fleet_display: FleetDisplay,
    #[serde(default, deserialize_with = "lenient")]
    pub ui_font: UiFont,
    #[serde(default, deserialize_with = "lenient")]
    pub mono_font: MonoFont,
    /// Card edges, the radar and other motion; off keeps the window idle.
    #[serde(default = "yes")]
    pub animations: bool,
    /// A release the person chose to skip: its new-version notice does not show again; a later release's does.
    #[serde(default)]
    pub skipped_version: Option<String>,
}

fn yes() -> bool {
    true
}

// Not derived: `bool` defaults to false, and a settings file without the key must read as sticky.
impl Default for GuiSettings {
    fn default() -> GuiSettings {
        GuiSettings {
            close_choice: None,
            theme: ThemeChoice::default(),
            last_pick_dir: None,
            sticky_alerts: true,
            active_persona: None,
            active_product: None,
            fleet_display: FleetDisplay::default(),
            ui_font: UiFont::default(),
            mono_font: MonoFont::default(),
            animations: true,
            skipped_version: None,
        }
    }
}

impl GuiSettings {
    /// A missing or unreadable file is just the defaults: settings must never stop the app from opening.
    pub fn load(path: &Path) -> GuiSettings {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, toml::to_string(self).map_err(std::io::Error::other)?)
    }

    /// Re-reads the file, applies `change` and saves, so a writer holding an older copy of the settings cannot
    /// overwrite a field another part of the window saved since.
    pub fn update(
        path: &Path,
        change: impl FnOnce(&mut GuiSettings),
    ) -> std::io::Result<GuiSettings> {
        let mut settings = GuiSettings::load(path);
        change(&mut settings);
        settings.save(path)?;
        Ok(settings)
    }
}

type Change = Arc<dyn Fn(&mut GuiSettings) + Send + Sync>;
type Done = Box<dyn FnOnce(std::io::Result<GuiSettings>) + Send>;

enum Job {
    Write {
        path: PathBuf,
        change: Change,
        done: Done,
    },
    Flush(mpsc::Sender<()>),
}

/// Saves `gui.toml` on one background thread: the home can be on NFS, where a write stalls the frame it happens in.
/// Writes keep the order they were asked in, so two changes never race their read-modify-write, and `current` sees a
/// change at once, before it reaches the disk.
pub struct SettingsWriter {
    tx: Mutex<mpsc::Sender<Job>>,
    /// Changes asked for and not yet written, per file, oldest first.
    pending: Arc<Mutex<HashMap<PathBuf, Vec<Change>>>>,
}

impl SettingsWriter {
    /// The one every window part shares, so all writes to a file go through one queue.
    pub fn shared() -> &'static SettingsWriter {
        static SHARED: OnceLock<SettingsWriter> = OnceLock::new();
        SHARED.get_or_init(SettingsWriter::start)
    }

    fn start() -> SettingsWriter {
        let (tx, rx) = mpsc::channel::<Job>();
        let pending: Arc<Mutex<HashMap<PathBuf, Vec<Change>>>> = Arc::default();
        let queue = pending.clone();
        std::thread::spawn(move || {
            for job in rx {
                match job {
                    Job::Write { path, change, done } => {
                        let result = GuiSettings::update(&path, |s| change(s));
                        // After the write: until then `current` applies the change itself (setting a field twice
                        // is harmless).
                        if let Some(list) = queue
                            .lock()
                            .expect("settings queue poisoned")
                            .get_mut(&path)
                        {
                            if !list.is_empty() {
                                list.remove(0);
                            }
                        }
                        done(result);
                    }
                    Job::Flush(ack) => {
                        let _ = ack.send(());
                    }
                }
            }
        });
        SettingsWriter {
            tx: Mutex::new(tx),
            pending,
        }
    }

    /// Applies `change` to the file in the background, through `GuiSettings::update`; `done` gets the outcome on
    /// the writer's thread.
    pub fn update(
        &self,
        path: &Path,
        change: impl Fn(&mut GuiSettings) + Send + Sync + 'static,
        done: impl FnOnce(std::io::Result<GuiSettings>) + Send + 'static,
    ) {
        let change: Change = Arc::new(change);
        self.pending
            .lock()
            .expect("settings queue poisoned")
            .entry(path.to_path_buf())
            .or_default()
            .push(change.clone());
        let job = Job::Write {
            path: path.to_path_buf(),
            change,
            done: Box::new(done),
        };
        let _ = self.tx.lock().expect("settings writer poisoned").send(job);
    }

    /// The file's settings with every change not yet written applied: what this process last asked for.
    pub fn current(&self, path: &Path) -> GuiSettings {
        let mut settings = GuiSettings::load(path);
        if let Some(list) = self
            .pending
            .lock()
            .expect("settings queue poisoned")
            .get(path)
        {
            for change in list {
                change(&mut settings);
            }
        }
        settings
    }

    /// Blocks until every write asked for so far is done: on exit, so a choice made just before closing is kept.
    pub fn flush(&self) {
        let (ack, wait) = mpsc::channel();
        if self
            .tx
            .lock()
            .expect("settings writer poisoned")
            .send(Job::Flush(ack))
            .is_ok()
        {
            let _ = wait.recv();
        }
    }
}
