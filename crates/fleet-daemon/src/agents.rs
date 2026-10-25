//! The agent lifecycle: spawn, answer, pause, resume, stop, and reconcile with what tmux really runs.
use crate::agent_config;
use crate::config::Config;
use crate::fleet::breakdown::{BreakdownAnswer, BreakdownError};
use crate::fleet::defs::FleetError;
use crate::fleet::ops::{agent_vars, ORCHESTRATOR_DIR};
use crate::footprint;
use crate::git::{self, GitError, Worktree};
use crate::hooks_cfg::{write_mcp_config, write_settings, RoleSettings, SettingsError};
use crate::orphans;
use crate::personas::{ambient_vars, PersonaEnv, PersonaError, Personas};
use crate::repo_rules::RepoRules;
use crate::repos::{RepoError, Repos, Resolved};
use crate::store::{RepoRow, Store, StoreError};
use crate::tmux::{Tmux, TmuxError};
use fleet_api::{
    project_id, Agent, AgentConfig, AgentId, AgentKind, ArchiveReport, BlockedReason, DiffReport,
    GateRun, LogEntry, Orphan, OrphanKind, ProjectState, RepoRequest, SpawnRequest, SpawnResponse,
    Status, StopCause, Workspace, WorkspaceDiff,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info, warn};

pub const SESSION_PREFIX: &str = "fleet-";
const MAX_PROMPT_BYTES: usize = 20_000;
/// The prompt's file in the project folder. Not on `claude`'s command line, which any local user can read in `ps`.
pub const PROMPT_FILE: &str = "PROMPT.md";
/// What the command line says instead of the prompt.
pub const PROMPT_POINTER: &str =
    "Your task is in PROMPT.md in this folder. Read it first, then do it.";
const MAX_DIFF_BYTES: usize = 1 << 20;
/// git's empty tree: the base of a repo the agent made itself, so its diff shows everything in it.
pub const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// Set on every agent's process so Claude Code keeps a `request_repo` call open while the owner decides. With
/// its defaults it gives up after 60 s, or moves the call to the background at 120 s and drops it after 300 s
/// of silence (measured, docs/superpowers/findings/mcp-spike.md).
pub const MCP_HOLD_ENV: [(&str, &str); 3] = [
    ("MCP_TOOL_TIMEOUT", "2400000"),
    ("CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT", "0"),
    ("CLAUDE_CODE_MCP_AUTO_BACKGROUND_MS", "3600000"),
];

/// Below the 40 min `MCP_TOOL_TIMEOUT`, so the daemon ends a hold before the client does: a decision made later is
/// then typed into the session instead of being written to a connection nobody reads any more.
const REPO_REQUEST_HOLD: Duration = Duration::from_secs(38 * 60);

pub trait Clock: Send + Sync {
    /// Unix seconds.
    fn now(&self) -> i64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }
}

#[derive(Clone)]
pub struct ServiceOptions {
    /// Program and leading args used to start Claude Code, e.g. `["claude"]`.
    pub claude: Vec<String>,
    /// Where hooks reach the daemon, e.g. `http://127.0.0.1:7878`.
    pub base_url: String,
    /// Seconds a card may stay Queued with no SessionStart before it is flagged as stuck.
    pub startup_grace_secs: i64,
    pub clock: Arc<dyn Clock>,
    /// Recorded on every agent as its owner; the local user until a team hub exists.
    pub owner: String,
    /// How long a `request_repo` call is held open waiting for the owner's decision.
    pub repo_request_hold: Duration,
    /// Test seam: `request_repo` waits on it between its write and registering its waiter, so a test can run a
    /// Stop in that gap. Always `None` in the daemon.
    #[doc(hidden)]
    pub request_gate: Option<Arc<RequestGate>>,
}

/// See `ServiceOptions::request_gate`: `reached` is notified when a request gets to the gap, which it leaves once
/// `open` is notified.
#[doc(hidden)]
#[derive(Default)]
pub struct RequestGate {
    pub reached: tokio::sync::Notify,
    pub open: tokio::sync::Notify,
}

impl ServiceOptions {
    pub fn new(base_url: &str) -> ServiceOptions {
        ServiceOptions {
            claude: vec!["claude".into()],
            base_url: base_url.into(),
            startup_grace_secs: 20,
            clock: Arc::new(SystemClock),
            owner: std::env::var("USER").unwrap_or_else(|_| "local".into()),
            repo_request_hold: REPO_REQUEST_HOLD,
            request_gate: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    #[error(transparent)]
    Repo(#[from] RepoError),
    #[error("prompt is empty")]
    EmptyPrompt,
    #[error("prompt is longer than {MAX_PROMPT_BYTES} bytes")]
    PromptTooLong,
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Tmux(#[from] TmuxError),
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("cannot write the project directory: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0} is not on the daemon's PATH and not an executable file; set claude_command in config.toml")]
    ClaudeNotFound(String),
    #[error(transparent)]
    Persona(#[from] PersonaError),
    #[error("{NO_PERSONA}")]
    NoPersona,
    #[error("the fleet-wide agent config is invalid: {0}")]
    AgentConfig(String),
    #[error("{ONE_REPO}")]
    OneRepo,
    #[error("set a test command for {0} first (Management → Personas)")]
    NoTestCommand(String),
    #[error(transparent)]
    Fleet(#[from] FleetError),
    #[error("a specialist is started by its orchestrator, not on its own")]
    SpecialistSpawn,
    #[error("pick at least one specialist for this project")]
    NoRoster,
}

const ONE_REPO: &str = "an orchestrated project has exactly one repo";
const NO_PERSONA: &str = "pick a persona for this project";

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("no such agent")]
    NotFound,
    #[error("agent is not running")]
    NotRunning,
    #[error("no such orphan")]
    NotAnOrphan,
    #[error("the project is not stopped; stop it first")]
    NotStopped,
    #[error("a live session is ended, not archived")]
    SessionNotArchived,
    /// A stopped agent whose kill failed: its claude still runs in the worktrees an archive would delete.
    #[error("the agent's tmux session is still running; end it before archiving")]
    SessionStillLive,
    #[error("{}", fleet_api::PLAN_CHANGED)]
    PlanChanged,
    /// Something an archive could not or would not remove; the text names it.
    #[error("{0}")]
    Removal(String),
    #[error("text is empty")]
    EmptyText,
    #[error(transparent)]
    Repo(#[from] RepoError),
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Tmux(#[from] TmuxError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("cannot write the project directory: {0}")]
    Io(#[from] std::io::Error),
    #[error("the fleet-wide agent config is invalid: {0}")]
    AgentConfig(String),
    #[error("no repo request is waiting for an answer")]
    NoPendingRequest,
    #[error("a repo request is already waiting for an answer")]
    RequestPending,
    #[error("the repo request was withdrawn")]
    RequestWithdrawn,
    #[error("no answer yet; the decision will be typed into this session when it is made")]
    NoAnswerYet,
    #[error(transparent)]
    Persona(#[from] PersonaError),
    /// A record from before personas that none has adopted yet: nothing may run for it under any login.
    #[error("{NO_PERSONA}")]
    NoPersona,
    #[error("{ONE_REPO}")]
    OneRepo,
    #[error("only an orchestrator proposes a breakdown")]
    NotAnOrchestrator,
    #[error("no breakdown is waiting for a decision")]
    NoPendingBreakdown,
    #[error("breakdown v{pending} is pending, not v{seen}; review it first")]
    StaleBreakdown { pending: u32, seen: u32 },
    #[error("a breakdown is already waiting for the owner's decision")]
    BreakdownPending,
    #[error(transparent)]
    Breakdown(#[from] BreakdownError),
    /// A fleet call refused for a reason the calling agent can act on; the text says which.
    #[error("{0}")]
    Fleet(String),
    /// The repo's tests failed or timed out, so nothing was pushed; the run is on the project record too.
    #[error("{}", gate_failure_text(.0))]
    GateFailed(GateRun),
}

/// How a failed gate reads to the orchestrator, which answers it by proposing a fix step.
pub fn gate_failure_text(run: &GateRun) -> String {
    let why = match run.exit_code {
        Some(code) => format!("exit {code}"),
        None => format!("timed out after {}s", run.duration_secs),
    };
    format!("tests failed ({why}):\n{}", run.output_tail)
}

/// The owner's decision on a `request_repo` call.
#[derive(Clone, Debug, PartialEq)]
pub enum RepoAnswer {
    Granted(Workspace),
    /// The reason, never empty.
    Refused(String),
}

/// A repo attached by `attach`, before the agent is told about it.
struct Attached {
    w: Workspace,
    warning: Option<String>,
    /// False when the project already had it.
    new: bool,
}

/// Claude Code settings inside a checkout that will not apply, because Claude starts one level above it.
/// A path to an executable file, or a name found as one on `PATH`, the way `execvp` looks it up.
fn is_runnable(program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let executable = |p: &Path| {
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if program.contains('/') {
        return executable(Path::new(program));
    }
    !program.is_empty()
        && std::env::var_os("PATH")
            .is_some_and(|path| std::env::split_paths(&path).any(|d| executable(&d.join(program))))
}

/// The deny and ask rules of every repo in `workspaces`, rewritten for the project folder (see `repo_rules`).
pub fn project_repo_rules(workspaces: &[Workspace]) -> RepoRules {
    let repos: Vec<(String, PathBuf)> = workspaces
        .iter()
        .map(|w| (folder(w), PathBuf::from(&w.worktree)))
        .collect();
    RepoRules::collect(repos.iter().map(|(f, p)| (f.as_str(), p.as_path())))
}

const START_A_REPO: &str = "To start a new repo, create a subfolder and run `git init` in it.";
const ASK_FOR_A_REPO: &str =
    "To work in an existing repo, call the `request_repo` tool (fleet MCP server); never clone or copy repos yourself.";

/// The subfolder a workspace lives in, relative to the project folder.
fn folder(w: &Workspace) -> String {
    Path::new(&w.worktree)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| w.repo.clone())
}

/// How the daemon's `CLAUDE.md` starts: an orphan folder whose `CLAUDE.md` does not is not taken for the daemon's.
pub const CLAUDE_MD_HEADER: &str =
    "# fleet-manager project\n\nThis folder is a project folder, not a git repository.";

/// `CLAUDE.md` for a project folder: the fleet-wide instructions, where the repos are, how to get more, plus an
/// import of each repo's own `CLAUDE.md`.
fn project_claude_md(instructions: &str, workspaces: &[Workspace]) -> String {
    let mut text = instructions_section(instructions);
    text.push_str(&repo_section(workspaces, true));
    text
}

/// The fleet-wide instructions as `CLAUDE.md` opens with them; empty when there are none.
pub(crate) fn instructions_section(instructions: &str) -> String {
    if instructions.trim().is_empty() {
        return String::new();
    }
    format!(
        "# Instructions for every agent\n\n{}\n\n",
        instructions.trim_end()
    )
}

/// Where the repos are, and with `more_repos` how to get others (an orchestrated project has its one), plus an
/// import of each repo's own `CLAUDE.md`.
pub(crate) fn repo_section(workspaces: &[Workspace], more_repos: bool) -> String {
    let mut text = String::from(CLAUDE_MD_HEADER);
    if workspaces.is_empty() {
        text.push_str(" This project has no repos yet.\n");
    } else {
        text.push_str(" Each repo of this project is a subfolder:\n\n");
        for w in workspaces {
            text.push_str(&format!("- `{}/` (branch `{}`)\n", folder(w), w.branch));
        }
        text.push_str("\nMake every change inside those subfolders and run git there. Do not create files directly in this folder.\n");
    }
    if more_repos {
        text.push_str(&format!("\n{START_A_REPO}\n{ASK_FOR_A_REPO}\n"));
    }
    for w in workspaces {
        if Path::new(&w.worktree).join("CLAUDE.md").is_file() {
            text.push_str(&format!("\n@{}/CLAUDE.md\n", folder(w)));
        }
    }
    text
}

/// Readable by the owner only: the prompt can hold anything the user typed.
pub(crate) fn write_prompt(workdir: &Path, prompt: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(workdir.join(PROMPT_FILE))?;
    f.write_all(prompt.as_bytes())?;
    f.write_all(b"\n")
}

fn write_claude_md(
    workdir: &Path,
    instructions: &str,
    workspaces: &[Workspace],
) -> std::io::Result<()> {
    std::fs::write(
        workdir.join("CLAUDE.md"),
        project_claude_md(instructions, workspaces),
    )
}

/// The fleet-wide config as it is on disk now: read on every spawn and attach so an edit applies to the next one.
pub(crate) fn fleet_config(dir: &Path) -> Result<AgentConfig, String> {
    agent_config::load_valid(dir).map_err(|e| e.to_string())
}

/// `argv` run through `env -u` for every ambient token and git credential setup. tmux can only add to a
/// session's environment, and its server keeps whatever the shell that first started it had, so this is the one way
/// to be sure the agent never logs in with anything that is not its persona's.
fn without_ambient_tokens(argv: Vec<String>) -> Vec<String> {
    let mut wrapped = vec!["env".to_string()];
    for k in ambient_vars() {
        wrapped.extend(["-u".to_string(), k.to_string()]);
    }
    wrapped.push("--".into());
    wrapped.extend(argv);
    wrapped
}

/// What spawn must remove again if a later step fails.
pub(crate) struct Undo {
    /// Each worktree made so far, with the base it hangs off.
    pub made: Vec<(PathBuf, Worktree)>,
    pub workdir: PathBuf,
    pub settings_dir: PathBuf,
}

/// tmux types a newline as Enter, which would submit half a line.
/// Control characters (escape sequences, Ctrl keys) would act on the agent's terminal rather than be read by it.
pub(crate) fn one_line(text: &str) -> String {
    let clean: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    clean.split_whitespace().collect::<Vec<_>>().join(" ")
}

enum MeasureJob {
    One(AgentId),
    All,
}

/// Everything the measuring task needs, so it never holds the service itself (the service owns its sender).
struct Measurer {
    store: Arc<Store>,
    state_dir: PathBuf,
    clock: Arc<dyn Clock>,
    /// Ids queued and not yet picked up: a second request for the same project adds nothing.
    queued: Arc<Mutex<HashSet<String>>>,
    /// Set from the request for a full pass until that pass is over, so refreshes meanwhile add nothing.
    all_pending: Arc<AtomicBool>,
    all_passes: Arc<AtomicUsize>,
    /// The clone cache's size as of the last full pass.
    clones_bytes: Arc<Mutex<Option<u64>>>,
}

impl Measurer {
    async fn run(self, mut jobs: mpsc::UnboundedReceiver<MeasureJob>) {
        while let Some(job) = jobs.recv().await {
            let (ids, full_pass) = match job {
                MeasureJob::One(id) => {
                    self.queued.lock().expect("queued poisoned").remove(&id.0);
                    (vec![id], false)
                }
                MeasureJob::All => match self.store.projects().await {
                    Ok(all) => (
                        all.into_iter()
                            .filter(|p| p.state != ProjectState::Archived)
                            .map(|p| p.id)
                            .collect(),
                        true,
                    ),
                    Err(e) => {
                        warn!(error = %e, "cannot list projects to measure");
                        self.all_pending.store(false, Ordering::SeqCst);
                        continue;
                    }
                },
            };
            for id in ids {
                if let Err(e) = self.measure_one(&id).await {
                    warn!(agent_id = %id.0, error = %e, "cannot measure project");
                }
            }
            if full_pass {
                self.measure_clones().await;
                self.all_passes.fetch_add(1, Ordering::SeqCst);
                // Cleared only now: a refresh that arrives mid-walk is folded into this pass, not queued behind it.
                self.all_pending.store(false, Ordering::SeqCst);
            }
        }
    }

    async fn measure_clones(&self) {
        let dir = self.state_dir.join("repos");
        match tokio::task::spawn_blocking(move || footprint::dir_bytes(&dir)).await {
            Ok(bytes) => *self.clones_bytes.lock().expect("clones poisoned") = Some(bytes),
            Err(e) => warn!(error = %e, "measuring the clone cache panicked"),
        }
    }

    async fn measure_one(&self, id: &AgentId) -> Result<(), StoreError> {
        let Some(p) = self.store.project(id).await? else {
            return Ok(());
        };
        let workdir = PathBuf::from(&p.workdir);
        // No folder, nothing to weigh: keep the last number rather than record zero.
        if p.state == ProjectState::Archived || !workdir.is_dir() {
            return Ok(());
        }
        let worktrees: Vec<(String, PathBuf)> = p
            .repos
            .iter()
            .map(|w| (w.repo.clone(), PathBuf::from(&w.worktree)))
            .collect();
        let daemon_files = self.state_dir.join("agents").join(&id.0);
        let now = self.clock.now();
        let fp = match tokio::task::spawn_blocking(move || {
            footprint::measure(&workdir, &worktrees, &daemon_files, now)
        })
        .await
        {
            Ok(fp) => fp,
            Err(e) => {
                warn!(agent_id = %id.0, error = %e, "measuring panicked");
                return Ok(());
            }
        };
        let total = fp.total;
        // An archive that finished during the walk has written the final footprint; this one is of what it removed.
        self.store
            .update_project(id, |p| {
                if p.state != ProjectState::Archived {
                    p.footprint = Some(fp)
                }
            })
            .await?;
        match self
            .store
            .update_agent(id, |a| a.disk_bytes = Some(total))
            .await
        {
            // A project whose agent record is gone (archived earlier) has only the footprint to update.
            Ok(_) | Err(StoreError::NotFound(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }
}

/// What the project records, the agents and the spawns in flight own: what is never an orphan.
#[derive(Default)]
pub(crate) struct Owned {
    pub(crate) workdirs: HashSet<PathBuf>,
    pub(crate) worktrees: Vec<PathBuf>,
    pub(crate) ids: HashSet<String>,
    pub(crate) sessions: HashSet<String>,
}

impl Owned {
    /// A folder or worktree is owned when it holds, or sits in, anything a record names.
    pub(crate) fn owns(&self, o: &Orphan) -> bool {
        let path = Path::new(&o.path);
        match o.kind {
            OrphanKind::ProjectFolder | OrphanKind::Worktree => self
                .workdirs
                .iter()
                .chain(&self.worktrees)
                .any(|d| d.starts_with(path) || path.starts_with(d)),
            OrphanKind::AgentFolder => path
                .file_name()
                .is_some_and(|n| self.ids.contains(n.to_string_lossy().as_ref())),
            OrphanKind::Session => self.sessions.contains(&o.path),
        }
    }
}

pub struct AgentService {
    pub(crate) cfg: Config,
    personas: Arc<Personas>,
    pub(crate) store: Arc<Store>,
    pub(crate) repos: Arc<Repos>,
    pub(crate) tmux: Tmux,
    pub(crate) opts: ServiceOptions,
    /// Agents mid-spawn, by id, with their project folder: from before the folder exists until the tmux session
    /// does. Reconcile must not call them lost, and the orphan scan must not call their folder or session orphans.
    spawning: Mutex<HashMap<String, PathBuf>>,
    /// Held from choosing a project directory until it exists, so two spawns never pick the same name. An orphan
    /// archive holds it while it re-checks and removes, so no new folder appears under it meanwhile.
    pub(crate) pick_lock: tokio::sync::Mutex<()>,
    /// Held while a project's workspace list is read, extended and written back (attach, adopt, archive), so two
    /// changes never both push from the same stale list.
    pub(crate) workspaces_lock: tokio::sync::Mutex<()>,
    /// Held while a repo request is opened or decided, so a grant never races the request it answers (or another
    /// decision on it). Decisions hold it through the attach, which may fetch.
    requests_lock: tokio::sync::Mutex<()>,
    /// The answer channel of each held `request_repo` call, by agent id. An entry may outlive its call (the client
    /// gave up, the hold ran out): sending on it then fails, which is how a decision knows to type instead.
    waiters: Mutex<HashMap<String, oneshot::Sender<RepoAnswer>>>,
    /// As `requests_lock`, for a breakdown proposed or decided: a decision never races the proposal it answers.
    pub(crate) breakdown_lock: tokio::sync::Mutex<()>,
    /// The answer channel of each held `propose_breakdown` call, by orchestrator id; the same rules as `waiters`.
    pub(crate) breakdown_waiters: Mutex<HashMap<String, oneshot::Sender<BreakdownAnswer>>>,
    /// Projects whose `open_pr` is past its breakdown check, by project id. A proposal or spawn for one is refused
    /// rather than held: the gate can run for half an hour, and `breakdown_lock` is every project's.
    pub(crate) gating: Mutex<HashSet<String>>,
    /// The queue of the one task that measures project folders, started by the first request (`new` may run
    /// outside a runtime).
    measure_tx: OnceLock<mpsc::UnboundedSender<MeasureJob>>,
    measure_queued: Arc<Mutex<HashSet<String>>>,
    measure_all_pending: Arc<AtomicBool>,
    measure_all_passes: Arc<AtomicUsize>,
    clones_bytes: Arc<Mutex<Option<u64>>>,
    /// True from a requested rescan until it has finished, so a refresh meanwhile adds nothing.
    rescan_pending: Arc<AtomicBool>,
    rescans: AtomicUsize,
    /// What the last scan found, with sizes filled in as the background walk reaches them.
    orphans: Arc<Mutex<Vec<Orphan>>>,
}

pub(crate) struct SpawningGuard<'a>(&'a Mutex<HashMap<String, PathBuf>>, String);

impl Drop for SpawningGuard<'_> {
    fn drop(&mut self) {
        self.0
            .lock()
            .expect("spawning set poisoned")
            .remove(&self.1);
    }
}

impl AgentService {
    pub fn new(
        cfg: Config,
        store: Arc<Store>,
        repos: Arc<Repos>,
        tmux: Tmux,
        opts: ServiceOptions,
    ) -> AgentService {
        let personas = Arc::new(Personas::new(
            store.clone(),
            cfg.personas_dir.clone(),
            cfg.claude_command.clone(),
            cfg.gh_command.clone(),
            cfg.projects_root.clone(),
            cfg.state_dir.clone(),
        ));
        AgentService {
            cfg,
            personas,
            store,
            repos,
            tmux,
            opts,
            spawning: Mutex::new(HashMap::new()),
            pick_lock: tokio::sync::Mutex::new(()),
            workspaces_lock: tokio::sync::Mutex::new(()),
            requests_lock: tokio::sync::Mutex::new(()),
            waiters: Mutex::new(HashMap::new()),
            breakdown_lock: tokio::sync::Mutex::new(()),
            breakdown_waiters: Mutex::new(HashMap::new()),
            gating: Mutex::new(HashSet::new()),
            measure_tx: OnceLock::new(),
            measure_queued: Arc::new(Mutex::new(HashSet::new())),
            measure_all_pending: Arc::new(AtomicBool::new(false)),
            measure_all_passes: Arc::new(AtomicUsize::new(0)),
            clones_bytes: Arc::new(Mutex::new(None)),
            rescan_pending: Arc::new(AtomicBool::new(false)),
            rescans: AtomicUsize::new(0),
            orphans: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Marks `id` as mid-spawn with `dir` as its folder until the guard drops (see `spawning`).
    pub(crate) fn spawning_guard(&self, id: &str, dir: &Path) -> SpawningGuard<'_> {
        self.spawning
            .lock()
            .expect("spawning set poisoned")
            .insert(id.to_string(), dir.to_path_buf());
        SpawningGuard(&self.spawning, id.to_string())
    }

    pub fn personas(&self) -> &Arc<Personas> {
        &self.personas
    }

    fn measure_queue(&self) -> &mpsc::UnboundedSender<MeasureJob> {
        self.measure_tx.get_or_init(|| {
            let (tx, rx) = mpsc::unbounded_channel();
            let m = Measurer {
                store: self.store.clone(),
                state_dir: self.cfg.state_dir.clone(),
                clock: self.opts.clock.clone(),
                queued: self.measure_queued.clone(),
                all_pending: self.measure_all_pending.clone(),
                all_passes: self.measure_all_passes.clone(),
                clones_bytes: self.clones_bytes.clone(),
            };
            tokio::spawn(m.run(rx));
            tx
        })
    }

    /// Queues a disk measurement of one project and returns at once; the result lands on its record and card.
    /// Must be called inside the tokio runtime.
    pub fn request_measure(&self, id: &AgentId) {
        let fresh = self
            .measure_queued
            .lock()
            .expect("queued poisoned")
            .insert(id.0.clone());
        if fresh {
            let _ = self.measure_queue().send(MeasureJob::One(id.clone()));
        }
    }

    /// Queues a measurement of every project that is not archived, and of the clone cache. A request made while a
    /// full pass is queued or running is folded into it: a refresh button must not stack walks of the whole disk.
    pub fn request_measure_all(&self) {
        if !self.measure_all_pending.swap(true, Ordering::SeqCst) {
            let _ = self.measure_queue().send(MeasureJob::All);
        }
    }

    /// Full measurement passes finished so far.
    pub fn measure_all_passes(&self) -> usize {
        self.measure_all_passes.load(Ordering::SeqCst)
    }

    /// The clone cache's size from the last full pass; None before the first.
    pub fn clones_bytes(&self) -> Option<u64> {
        *self.clones_bytes.lock().expect("clones poisoned")
    }

    /// `rescan_orphans` in the background, unless one is already queued or running.
    /// Must be called inside the tokio runtime.
    pub fn request_rescan(self: &Arc<Self>) {
        if self.rescan_pending.swap(true, Ordering::SeqCst) {
            return;
        }
        let svc = self.clone();
        tokio::spawn(async move {
            svc.rescan_orphans().await;
            svc.rescans.fetch_add(1, Ordering::SeqCst);
            svc.rescan_pending.store(false, Ordering::SeqCst);
        });
    }

    /// Rescans started through `request_rescan` that have finished.
    pub fn rescans_run(&self) -> usize {
        self.rescans.load(Ordering::SeqCst)
    }

    /// What the last scan found: at startup, and after each `rescan_orphans`.
    pub fn orphans(&self) -> Vec<Orphan> {
        self.orphans.lock().expect("orphans poisoned").clone()
    }

    /// What the records and the spawns in flight own now. The spawn set is read before the store: a spawn
    /// registers before its folder exists and leaves the set only after its record is stored, so anything found on
    /// disk or in tmux before this call is owned by one or the other.
    pub(crate) async fn owned_now(&self) -> Result<Owned, ServiceError> {
        let spawning: Vec<(String, PathBuf)> = self
            .spawning
            .lock()
            .expect("spawning set poisoned")
            .iter()
            .map(|(id, w)| (id.clone(), w.clone()))
            .collect();
        let mut owned = Owned::default();
        for (id, workdir) in spawning {
            owned
                .sessions
                .insert(format!("{SESSION_PREFIX}{}", &id[..8.min(id.len())]));
            owned.ids.insert(id);
            owned.workdirs.insert(workdir);
        }
        for p in self.store.projects().await? {
            owned.workdirs.insert(PathBuf::from(&p.workdir));
            owned
                .worktrees
                .extend(p.repos.iter().map(|w| PathBuf::from(&w.worktree)));
            owned.ids.insert(p.id.0);
        }
        for a in self.store.snapshot().await?.agents {
            owned.workdirs.insert(PathBuf::from(&a.workdir));
            owned
                .worktrees
                .extend(a.workspaces.iter().map(|w| PathBuf::from(&w.worktree)));
            owned.sessions.insert(a.tmux_session);
            owned.ids.insert(a.id.0);
        }
        Ok(owned)
    }

    /// Every tmux session with the daemon's prefix that no agent owns, spawning ones included.
    async fn orphan_sessions(&self) -> Result<Vec<Orphan>, ServiceError> {
        // Listed before the owners are read: a session is started only after its agent is stored.
        let live = self.tmux.list_sessions(SESSION_PREFIX).await?;
        let owned = self.owned_now().await?;
        Ok(orphans::orphan_sessions(live, &owned.sessions))
    }

    async fn find_orphans(&self) -> Result<Vec<Orphan>, ServiceError> {
        let bases: Vec<PathBuf> = self
            .store
            .repos()
            .await?
            .into_iter()
            .map(|r| r.base)
            .collect();
        let before = self.owned_now().await?;
        let root = self.cfg.projects_root.clone();
        let agents_dir = self.cfg.state_dir.join("agents");
        let mut found = tokio::task::spawn_blocking(move || {
            orphans::scan(&root, &agents_dir, &bases, &before.workdirs, &before.ids)
        })
        .await
        .map_err(std::io::Error::other)?;
        // A spawn can make its folder, and store its record, while the scan walks: checked again once it is done.
        let after = self.owned_now().await?;
        found.retain(|o| !after.owns(o));
        found.extend(self.orphan_sessions().await?);
        Ok(found)
    }

    /// Scans again and replaces the cached list. Sizes are walked afterwards in the background (one folder at a
    /// time, off the async threads), so this returns after the directory names and a few `git` calls.
    /// Must be called inside the tokio runtime.
    pub async fn rescan_orphans(&self) {
        let found = match self.find_orphans().await {
            Ok(f) => f,
            Err(e) => {
                warn!(error = %e, "cannot scan for orphans");
                return;
            }
        };
        let to_weigh: Vec<(String, PathBuf)> = found
            .iter()
            .filter(|o| o.kind != OrphanKind::Session)
            .map(|o| (o.id.clone(), PathBuf::from(&o.path)))
            .collect();
        if !found.is_empty() {
            info!(count = found.len(), "orphans found");
        }
        *self.orphans.lock().expect("orphans poisoned") = found;
        let cache = self.orphans.clone();
        tokio::spawn(async move {
            for (id, path) in to_weigh {
                let Ok(bytes) =
                    tokio::task::spawn_blocking(move || footprint::dir_bytes(&path)).await
                else {
                    continue;
                };
                // The list may have been replaced by a newer scan meanwhile; an id that is gone has no row.
                if let Some(o) = cache
                    .lock()
                    .expect("orphans poisoned")
                    .iter_mut()
                    .find(|o| o.id == id)
                {
                    o.bytes = Some(bytes);
                }
            }
        });
    }

    /// Kills a live tmux session that no agent owns. Checked against tmux, the agents and the spawns in flight now,
    /// not the cached scan, so a session an agent has just been given is never killed on a stale list.
    pub async fn end_session(&self, session: &str) -> Result<(), ServiceError> {
        let is_orphan = self
            .orphan_sessions()
            .await?
            .iter()
            .any(|o| o.path == session);
        if !is_orphan {
            return Err(ServiceError::NotAnOrphan);
        }
        self.tmux.kill(session).await?;
        info!(session, "orphan session ended");
        self.rescan_orphans().await;
        Ok(())
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    pub fn options(&self) -> &ServiceOptions {
        &self.opts
    }

    pub(crate) fn agent_dir(&self, id: &str) -> PathBuf {
        self.cfg.state_dir.join("agents").join(id)
    }

    pub async fn agent(&self, id: &AgentId) -> Result<Agent, ServiceError> {
        self.find(id).await
    }

    /// What pickers and `list_repos` offer, all or one persona's: repos owned by a project are left out (see
    /// `Repos::pickable`).
    pub async fn remembered_repos(
        &self,
        persona: Option<&str>,
    ) -> Result<Vec<RepoRow>, ServiceError> {
        Ok(self.repos.pickable(persona).await?)
    }

    pub(crate) async fn find(&self, id: &AgentId) -> Result<Agent, ServiceError> {
        let snap = self.store.snapshot().await?;
        snap.agents
            .into_iter()
            .find(|a| &a.id == id)
            .ok_or(ServiceError::NotFound)
    }

    /// First free `<slug>`, `<slug>-2`, ...: no such project folder and no `<prefix>/<name>` branch in any remembered
    /// repo, since stopping and archiving keep branches and any of those repos may be attached to this project later.
    async fn free_project_name(&self, slug: &str) -> Result<String, SpawnError> {
        let repos = self.store.repos().await?;
        for n in 1.. {
            let name = if n == 1 {
                slug.to_string()
            } else {
                format!("{slug}-{n}")
            };
            if self.cfg.projects_root.join(&name).exists() {
                continue;
            }
            let branch = format!("{}/{name}", self.cfg.branch_prefix);
            let mut taken = false;
            for r in &repos {
                if git::branch_exists(&r.base, &branch).await? {
                    taken = true;
                    break;
                }
            }
            if !taken {
                return Ok(name);
            }
        }
        unreachable!("the loop ends by returning")
    }

    /// `spawn_reporting` without its warnings.
    pub async fn spawn(&self, req: SpawnRequest) -> Result<Agent, SpawnError> {
        Ok(self.spawn_reporting(req).await?.agent)
    }

    /// Starts an agent on a new project folder with zero or more repos. Undoes every step on failure, so a failed
    /// spawn leaves no worktree, branch, project folder, settings file, card or tmux session behind. What went
    /// wrong without stopping it comes back as warnings, for the user to see.
    #[tracing::instrument(skip_all, fields(agent_id = tracing::field::Empty))]
    pub async fn spawn_reporting(&self, req: SpawnRequest) -> Result<SpawnResponse, SpawnError> {
        let prompt = req.prompt.trim();
        if prompt.is_empty() {
            return Err(SpawnError::EmptyPrompt);
        }
        if prompt.len() > MAX_PROMPT_BYTES {
            return Err(SpawnError::PromptTooLong);
        }
        // Without it the session starts, its command fails at once and the card goes Stopped seconds later, with
        // nothing saying why.
        let program = self
            .opts
            .claude
            .first()
            .map(String::as_str)
            .unwrap_or_default();
        if !is_runnable(program) {
            return Err(SpawnError::ClaudeNotFound(program.to_string()));
        }
        if req.persona.is_empty() {
            return Err(SpawnError::NoPersona);
        }
        // Asked first, so a broken login costs nothing either.
        let persona = self.personas.ready_env(&req.persona).await?;
        let fleet = fleet_config(&self.cfg.agent_config_dir).map_err(SpawnError::AgentConfig)?;
        let orchestrator = self.orchestrator_def(&req)?;
        // Every source is resolved before anything is made, so a typo in the third repo costs nothing.
        // Two checkouts of one repo are one workspace: the first wins, as repeated names always did.
        let mut rows: Vec<Resolved> = Vec::with_capacity(req.repos.len());
        for input in &req.repos {
            let resolved = self.repos.resolve(input.trim(), &persona).await?;
            if !rows.iter().any(|r| r.row.name == resolved.row.name) {
                rows.push(resolved);
            }
        }
        let plan = orchestrator
            .map(|def| self.orchestrator_plan(def, &req, &rows))
            .transpose()?;

        let id = uuid::Uuid::new_v4().simple().to_string();
        tracing::Span::current().record("agent_id", id.as_str());
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let slug = git::slugify(req.task_slug.as_deref().unwrap_or(prompt));
        let session = format!("{SESSION_PREFIX}{}", &id[..8]);

        // The project folder is a plain folder under the projects root: Claude's trust is inherited from the
        // trusted root, which a git worktree used directly as cwd would not get. Once it exists no other spawn
        // can pick its name, so the lock is not held through the (possibly fetching) attaches below.
        let (workdir, name, _guard) = {
            let _pick = self.pick_lock.lock().await;
            let name = self.free_project_name(&slug).await?;
            let workdir = self.cfg.projects_root.join(&name);
            std::fs::create_dir_all(&self.cfg.projects_root)?;
            // Registered before the folder exists and dropped only after the record is stored (or the folder undone),
            // so an orphan scan never finds the folder with neither a record nor this entry.
            let guard = self.spawning_guard(&id, &workdir);
            // Not create_dir_all: a folder that appeared meanwhile is not ours, and undo deletes this one.
            std::fs::create_dir(&workdir)?;
            (workdir, name, guard)
        };
        let branch = format!("{}/{name}", self.cfg.branch_prefix);
        let settings_dir = self.agent_dir(&id);
        let mut undo = Undo {
            made: vec![],
            workdir: workdir.clone(),
            settings_dir: settings_dir.clone(),
        };
        // The agent's cwd: the project folder for a solo agent, its own folder inside it for an orchestrator.
        let cwd = match &plan {
            Some(_) => workdir.join(ORCHESTRATOR_DIR),
            None => workdir.clone(),
        };
        if let Err(e) = std::fs::create_dir_all(&cwd) {
            self.undo(&undo, None).await;
            return Err(e.into());
        }

        let mut workspaces = Vec::with_capacity(rows.len());
        let mut warnings = Vec::new();
        for Resolved { row, start } in &rows {
            let (wt, warning) = match self
                .repos
                .attach(row, start, &cwd.join(&row.name), &branch, &persona)
                .await
            {
                Ok(x) => x,
                Err(e) => {
                    self.undo(&undo, None).await;
                    return Err(e.into());
                }
            };
            if let Some(w) = warning {
                warn!(repo = %row.name, warning = %w, "repo attached with a warning");
                warnings.push(one_line(&w));
            }
            workspaces.push(Workspace {
                repo: row.name.clone(),
                branch: wt.branch.clone(),
                worktree: wt.path.to_string_lossy().into_owned(),
                base: wt.base.clone(),
                from_branch: wt.from_branch.clone(),
                has_changes: false,
                owned: false,
            });
            undo.made.push((row.base.clone(), wt));
        }

        let repo_rules = project_repo_rules(&workspaces);
        for note in &repo_rules.not_applied {
            warn!(note = %note, "repo setting not applied");
            warnings.push(note.clone());
        }
        let written = write_settings(
            &settings_dir,
            &self.opts.base_url,
            &AgentId(id.clone()),
            &repo_rules,
            &fleet.settings,
            &plan.as_ref().map(|p| p.settings()).unwrap_or_default(),
        )
        .and_then(|s| {
            Ok((
                s,
                write_mcp_config(
                    &settings_dir,
                    &self.opts.base_url,
                    &AgentId(id.clone()),
                    &token,
                )?,
            ))
        });
        let (settings, mcp) = match written {
            Ok(p) => p,
            Err(e) => {
                self.undo(&undo, None).await;
                return Err(e.into());
            }
        };
        // The agent starts one level above the checkouts, so without this it would write files next to a repo
        // instead of in it (seen on the real CLI). Claude also loads no CLAUDE.md from one level down (measured),
        // so each repo's own is imported. The file belongs to the daemon, not to any repo.
        let files = match &plan {
            Some(p) => self.write_orchestrator_files(p, &cwd, &fleet.instructions, &workspaces),
            None => {
                write_claude_md(&workdir, &fleet.instructions, &workspaces).map_err(SpawnError::Io)
            }
        }
        .and_then(|()| Ok(write_prompt(&cwd, prompt)?));
        if let Err(e) = files {
            self.undo(&undo, None).await;
            return Err(e);
        }

        // Stored before tmux starts: SessionStart can arrive within milliseconds of the process starting.
        let agent = Agent {
            id: AgentId(id.clone()),
            task: prompt
                .lines()
                .next()
                .unwrap_or(prompt)
                .chars()
                .take(120)
                .collect(),
            workdir: cwd.to_string_lossy().into_owned(),
            workspaces,
            tmux_session: session.clone(),
            status: Status::Queued,
            activity: "starting".into(),
            status_since: self.opts.clock.now(),
            has_changes: false,
            paused: false,
            owner: self.opts.owner.clone(),
            blocked_reason: None,
            repo_request: None,
            stop_cause: None,
            disk_bytes: None,
            persona: persona.id.clone(),
            kind: if plan.is_some() {
                AgentKind::Orchestrator
            } else {
                AgentKind::Solo
            },
            project: None,
            role: plan.as_ref().map(|p| p.def.name.clone()),
            step: None,
        };
        if let Err(e) = self.store.insert_agent(&agent, &token).await {
            self.undo(&undo, None).await;
            return Err(e.into());
        }
        if let Some(p) = &plan {
            if let Err(e) = self.record_orchestrator(&agent.id, p, &workdir).await {
                self.undo(&undo, Some((&agent.id, &e.to_string()))).await;
                return Err(e.into());
            }
        }

        let vars = agent_vars(&persona, agent.kind);
        let launched = self
            .launch(&session, &cwd, &settings, &mcp, &token, &vars)
            .await;
        if let Err(e) = launched {
            self.undo(&undo, Some((&agent.id, &e.to_string()))).await;
            return Err(e.into());
        }
        // Spans are not written to daemon.log, so the event carries the agent id itself.
        info!(agent_id = %id, project = %name, repos = agent.workspaces.len(), "agent spawned");
        Ok(SpawnResponse { agent, warnings })
    }

    /// Starts `claude` in a new tmux session in `cwd`, with the agent's token and env. On an error no session is left.
    pub(crate) async fn launch(
        &self,
        session: &str,
        cwd: &Path,
        settings: &Path,
        mcp: &Path,
        token: &str,
        vars: &[(String, String)],
    ) -> Result<(), TmuxError> {
        // The prompt itself is in PROMPT_FILE; the pointer is one argv element after `--`, never read as a flag.
        let mut argv = self.opts.claude.clone();
        argv.extend([
            "--settings".into(),
            settings.to_string_lossy().into_owned(),
            "--mcp-config".into(),
            mcp.to_string_lossy().into_owned(),
            "--".into(),
            PROMPT_POINTER.into(),
        ]);
        let argv = without_ambient_tokens(argv);
        let mut env = vec![("FLEET_TOKEN", token)];
        env.extend(MCP_HOLD_ENV);
        env.extend(vars.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        self.tmux.new_session(session, cwd, &env, &argv).await?;
        // `env -u` covers the agent's own process only; a window the user opens in the session must not get the
        // tokens either.
        let ambient: Vec<&str> = ambient_vars().collect();
        if let Err(e) = self.tmux.drop_environment(session, &ambient).await {
            // An agent that exited at once took its session (and maybe the server) with it: no window can be opened
            // there, and reconcile reports it stopped like any other.
            if self.tmux.has_session(session).await.unwrap_or(true) {
                let _ = self.tmux.kill(session).await;
                return Err(e);
            }
        }
        Ok(())
    }

    /// Only ever runs before the agent's process exists, so the project folder holds nothing but what spawn made.
    /// `stored` is the agent whose card and record were already stored, with what went wrong: the card goes, and the
    /// record (never deleted) is finished in the same write, so no Running record outlives the failed spawn. It is
    /// finished after the removals, so it can name what could not be removed.
    pub(crate) async fn undo(&self, u: &Undo, stored: Option<(&AgentId, &str)>) {
        let mut kept = Vec::new();
        let remove_dir = |dir: &Path, kept: &mut Vec<String>| match std::fs::remove_dir_all(dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                kept.push(format!("{} ({e})", dir.display()))
            }
            _ => {}
        };
        remove_dir(&u.settings_dir, &mut kept);
        let mut failed = Vec::new();
        for (base, wt) in u.made.iter().rev() {
            let _base = self.repos.lock_base(base).await;
            if git::remove(base, wt).await.is_err() {
                failed.push((base, wt));
            }
        }
        remove_dir(&u.workdir, &mut kept);
        // With its folder gone a worktree that git would not remove is only an entry to prune, so it gets one more
        // try; what is still left then is in the base, not the project folder.
        for (base, wt) in failed {
            let _base = self.repos.lock_base(base).await;
            let left = match git::remove_worktree(base, wt).await {
                Err(e) => Some((
                    format!(
                        "branch {} and the worktree entry for {} in {}",
                        wt.branch,
                        wt.path.display(),
                        base.display()
                    ),
                    e,
                )),
                Ok(()) => git::remove(base, wt)
                    .await
                    .err()
                    .map(|e| (format!("branch {} in {}", wt.branch, base.display()), e)),
            };
            if let Some((what, e)) = left {
                kept.push(format!("{what} ({})", one_line(&e.to_string())));
            }
        }
        if !kept.is_empty() {
            warn!(kept = %kept.join("; "), "a failed spawn left files behind");
        }
        let Some((id, error)) = stored else {
            return;
        };
        let now = self.opts.clock.now();
        let line = format!("spawn failed: {}", one_line(error));
        let kept = (!kept.is_empty()).then(|| format!("could not remove {}", kept.join("; ")));
        let r = self
            .store
            .archive_project(id, move |p| {
                p.state = ProjectState::Archived;
                p.archived_at = Some(now);
                p.archive = Some(ArchiveReport {
                    removed: vec![],
                    freed: 0,
                    kept,
                });
                p.log.push(LogEntry {
                    at: now,
                    text: line,
                });
            })
            .await;
        if let Err(e) = r {
            warn!(agent_id = %id.0, error = %e, "cannot finish the failed spawn's record");
        }
    }

    /// The environment of the persona a project runs under, checked again now; a record from before personas that
    /// none has adopted has none.
    pub(crate) async fn persona_of(&self, a: &Agent) -> Result<PersonaEnv, ServiceError> {
        if a.persona.is_empty() {
            return Err(ServiceError::NoPersona);
        }
        Ok(self.personas.ready_env(&a.persona).await?)
    }

    /// Adds a repo (name, path or URL) to a running or finished project as `<workdir>/<name>` on the project's
    /// branch, and tells the agent. A repo the project already has is returned unchanged.
    pub async fn attach_repo(&self, id: &AgentId, input: &str) -> Result<Workspace, ServiceError> {
        let Attached { w, warning, new } = self.attach(id, input).await?;
        if !new {
            return Ok(w);
        }
        info!(agent_id = %id.0, repo = %w.repo, "repo attached");
        self.log_project(id, format!("repo {} added", w.repo)).await;
        let warning = warning.map(|w| one_line(&w));
        let mut line = format!(
            "fleet-manager: repo {} is now available at {} (branch {}).",
            w.repo, w.worktree, w.branch
        );
        if let Some(warn) = &warning {
            line.push_str(&format!(" Warning: {warn}"));
        }
        let typed = match self.find(id).await {
            Ok(a) => self.type_line(&a, &line).await,
            Err(_) => false,
        };
        if !typed {
            if let Some(w) = &warning {
                warn!(agent_id = %id.0, warning = %w, "repo added with a warning");
            }
        }
        Ok(w)
    }

    /// Types `line` into the session of `a` (as read by the caller) if it is running and nothing but its input box
    /// can take the line; returns whether it did. tmux sends the line with Enter: on a permission prompt that picks
    /// the highlighted "Yes", and at the startup trust dialog (Queued, before any hook) it picks "No, exit" and ends
    /// the agent. Skipping is safe: `CLAUDE.md` lists the repos and `list_repos` answers.
    pub(crate) async fn type_line(&self, a: &Agent, line: &str) -> bool {
        let reads_text = match a.status {
            Status::Working | Status::Review => true,
            // Both wait on a held call that has returned or will: the agent is at its input box.
            Status::NeedsYou => matches!(
                a.blocked_reason,
                Some(BlockedReason::RepoAccess | BlockedReason::Breakdown)
            ),
            // Talking is not produced yet; until it is, nothing is known about what its screen shows.
            Status::Queued | Status::Talking | Status::Stopped => false,
        };
        if !reads_text {
            debug!(agent_id = %a.id.0, status = ?a.status, blocked_reason = ?a.blocked_reason, %line, "not typing into agent, a dialog may be up");
            return false;
        }
        if !self
            .tmux
            .has_session(&a.tmux_session)
            .await
            .unwrap_or(false)
        {
            return false;
        }
        if let Err(e) = self.tmux.send_literal(&a.tmux_session, line).await {
            warn!(agent_id = %a.id.0, error = %e, "cannot type into the agent's session");
            return false;
        }
        true
    }

    /// The attach behind `attach_repo` and `grant_repo`, which tell the agent in different ways.
    #[tracing::instrument(skip_all, fields(agent_id = %id.0))]
    async fn attach(&self, id: &AgentId, input: &str) -> Result<Attached, ServiceError> {
        // Before the lock: the logins are asked through the CLIs, and a project's persona never changes.
        let persona = self.persona_of(&self.find(id).await?).await?;
        let fleet = fleet_config(&self.cfg.agent_config_dir).map_err(ServiceError::AgentConfig)?;
        let _ws = self.workspaces_lock.lock().await;
        let a = self.find(id).await?;
        // Its CLAUDE.md and settings are its role's, which the solo ones written below would replace.
        if a.kind != AgentKind::Solo {
            return Err(ServiceError::OneRepo);
        }
        let input = input.trim();
        if let Some(w) = a.workspaces.iter().find(|w| w.repo == input) {
            return Ok(Attached {
                w: w.clone(),
                warning: None,
                new: false,
            });
        }
        let Resolved { row, start } = self.repos.resolve(input, &persona).await?;
        if let Some(w) = a.workspaces.iter().find(|w| w.repo == row.name) {
            return Ok(Attached {
                w: w.clone(),
                warning: None,
                new: false,
            });
        }
        let workdir = PathBuf::from(&a.workdir);
        // An owned repo is on whatever branch the agent chose, so only an attached one names the project branch.
        let branch = match a.workspaces.iter().find(|w| !w.owned) {
            Some(w) => w.branch.clone(),
            None => {
                let name = workdir
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                format!("{}/{name}", self.cfg.branch_prefix)
            }
        };
        let (wt, warning) = self
            .repos
            .attach(&row, &start, &workdir.join(&row.name), &branch, &persona)
            .await?;
        let w = Workspace {
            repo: row.name.clone(),
            branch: wt.branch.clone(),
            worktree: wt.path.to_string_lossy().into_owned(),
            base: wt.base.clone(),
            from_branch: wt.from_branch.clone(),
            has_changes: false,
            owned: false,
        };
        let pushed = w.clone();
        if let Err(e) = self
            .store
            .update_agent(id, move |x| x.workspaces.push(pushed))
            .await
        {
            // Killed meanwhile: the worktree has no card to be removed with.
            let _base = self.repos.lock_base(&row.base).await;
            let _ = git::remove(&row.base, &wt).await;
            return Err(e.into());
        }
        let mut all = a.workspaces.clone();
        all.push(w.clone());
        if let Err(e) = write_claude_md(&workdir, &fleet.instructions, &all) {
            warn!(agent_id = %id.0, path = %workdir.join("CLAUDE.md").display(), error = %e, "cannot rewrite CLAUDE.md");
        }
        // The new repo's deny and ask rules. Whether a running claude reloads its --settings file is not
        // documented, so they are certain to apply from the agent's next start.
        if let Err(e) = write_settings(
            &self.agent_dir(&id.0),
            &self.opts.base_url,
            id,
            &project_repo_rules(&all),
            &fleet.settings,
            &RoleSettings::default(),
        ) {
            warn!(agent_id = %id.0, error = %e, "cannot rewrite the agent's settings");
        }
        Ok(Attached {
            w,
            warning,
            new: true,
        })
    }

    /// The workspace for `input` if the project already has it, matched only against what is already known:
    /// an ask must not clone or remember anything before it is granted.
    async fn existing_workspace(
        &self,
        a: &Agent,
        input: &str,
    ) -> Result<Option<Workspace>, ServiceError> {
        let canon = |p: &str| std::fs::canonicalize(p).ok();
        let wanted = canon(input);
        if let Some(w) = a
            .workspaces
            .iter()
            .find(|w| w.repo == input || (wanted.is_some() && canon(&w.worktree) == wanted))
        {
            return Ok(Some(w.clone()));
        }
        let rows = self.store.repos_for(&a.persona).await?;
        let name = rows
            .iter()
            .find(|r| r.location == input || (wanted.is_some() && canon(&r.location) == wanted))
            .map(|r| &r.name);
        Ok(name
            .and_then(|n| a.workspaces.iter().find(|w| &w.repo == n))
            .cloned())
    }

    /// The agent's `request_repo`: answers at once when the project has the repo, otherwise puts the card in
    /// Needs you and holds until the owner decides (or the hold runs out, see `REPO_REQUEST_HOLD`).
    pub async fn request_repo(
        &self,
        id: &AgentId,
        repo: &str,
        why: &str,
    ) -> Result<RepoAnswer, ServiceError> {
        // Both end up on the card and in a line typed into the session.
        let (repo, why) = (one_line(repo), one_line(why));
        if repo.is_empty() {
            return Err(ServiceError::EmptyText);
        }
        let rx = {
            let _req = self.requests_lock.lock().await;
            let a = self.find(id).await?;
            // Nothing is left to answer a stopped agent, and a request must not pull its card out of Stopped.
            if a.status == Status::Stopped {
                return Err(ServiceError::NotRunning);
            }
            if let Some(w) = self.existing_workspace(&a, &repo).await? {
                return Ok(RepoAnswer::Granted(w));
            }
            let now = self.opts.clock.now();
            let mut pending = false;
            let mut stopped = false;
            let request = RepoRequest {
                repo: repo.clone(),
                why,
            };
            self.store
                .update_agent(id, |x| {
                    // Re-checked in the write: the session can end after the read above.
                    if x.status == Status::Stopped {
                        stopped = true;
                        return;
                    }
                    if x.repo_request.is_some() {
                        pending = true;
                        return;
                    }
                    if x.status != Status::NeedsYou {
                        x.status_since = now;
                    }
                    x.status = Status::NeedsYou;
                    x.blocked_reason = Some(BlockedReason::RepoAccess);
                    x.activity = format!("wants repo {repo}");
                    x.repo_request = Some(request);
                })
                .await?;
            if stopped {
                return Err(ServiceError::NotRunning);
            }
            if pending {
                return Err(ServiceError::RequestPending);
            }
            if let Some(gate) = &self.opts.request_gate {
                gate.reached.notify_one();
                gate.open.notified().await;
            }
            let (tx, rx) = oneshot::channel();
            self.waiters
                .lock()
                .expect("waiters poisoned")
                .insert(id.0.clone(), tx);
            // Stop removes the waiters once, after its Stopped write, without this lock: a waiter registered after
            // that removal would be held for the whole hold. Read after registering, either this sees Stopped or
            // the removal comes after the registration.
            let now_stopped = self.find(id).await.map(|a| a.status == Status::Stopped);
            if !matches!(now_stopped, Ok(false)) {
                self.waiters.lock().expect("waiters poisoned").remove(&id.0);
                now_stopped?;
                return Err(ServiceError::NotRunning);
            }
            rx
        };
        match tokio::time::timeout(self.opts.repo_request_hold, rx).await {
            Ok(Ok(answer)) => Ok(answer),
            Ok(Err(_)) => Err(ServiceError::RequestWithdrawn),
            Err(_) => Err(ServiceError::NoAnswerYet),
        }
    }

    /// Attaches `input` (what was asked, or the owner's edit of it) and answers the request. A failed attach
    /// leaves the request pending, so the owner can fix the target and grant again.
    pub async fn grant_repo(&self, id: &AgentId, input: &str) -> Result<Workspace, ServiceError> {
        let _req = self.requests_lock.lock().await;
        if self.find(id).await?.repo_request.is_none() {
            return Err(ServiceError::NoPendingRequest);
        }
        let Attached { w, warning, .. } = self.attach(id, input).await?;
        info!(agent_id = %id.0, repo = %w.repo, "repo granted");
        self.log_project(id, format!("repo {} granted", w.repo))
            .await;
        // Read before settle moves the card to Working: the fallback below may only type onto the request itself.
        let before = self.find(id).await?;
        self.settle(id, format!("granted repo {}", w.repo)).await?;
        let warning = warning.map(|w| one_line(&w));
        if self.answer_held(id, RepoAnswer::Granted(w.clone())) {
            if let Some(w) = &warning {
                warn!(agent_id = %id.0, warning = %w, "repo granted with a warning");
            }
        } else {
            let mut line = format!(
                "fleet-manager: granted {} at {} (branch {})",
                w.repo, w.worktree, w.branch
            );
            if let Some(warn) = &warning {
                line.push_str(&format!(" Warning: {warn}"));
            }
            self.type_line(&before, &line).await;
        }
        Ok(w)
    }

    pub async fn refuse_repo(&self, id: &AgentId, reason: &str) -> Result<(), ServiceError> {
        let _req = self.requests_lock.lock().await;
        let Some(request) = self.find(id).await?.repo_request else {
            return Err(ServiceError::NoPendingRequest);
        };
        let reason = match one_line(reason) {
            r if r.is_empty() => "no reason given".to_string(),
            r => r,
        };
        let before = self.find(id).await?;
        self.settle(id, format!("refused repo {}", request.repo))
            .await?;
        // What the agent asked for may be a URL with credentials in it.
        info!(agent_id = %id.0, repo = %crate::repos::log_name(&request.repo), "repo request refused");
        self.log_project(id, format!("repo request refused: {reason}"))
            .await;
        if !self.answer_held(id, RepoAnswer::Refused(reason.clone())) {
            self.type_line(
                &before,
                &format!("fleet-manager: refused {}: {reason}", request.repo),
            )
            .await;
        }
        Ok(())
    }

    /// Adds a line to the project's history. The record is secondary to what happened, so a failure is only logged.
    pub(crate) async fn log_project(&self, id: &AgentId, text: String) {
        if let Err(e) = self
            .store
            .project_log(id, self.opts.clock.now(), &text)
            .await
        {
            warn!(agent_id = %id.0, error = %e, "cannot log to the project record");
        }
    }

    /// Ends a decided request on the card; the hooks move the status on from here.
    pub(crate) async fn settle(&self, id: &AgentId, activity: String) -> Result<(), ServiceError> {
        let now = self.opts.clock.now();
        self.store
            .update_agent(id, |x| {
                x.repo_request = None;
                if x.status == Status::Stopped {
                    return;
                }
                if x.status != Status::Working {
                    x.status_since = now;
                }
                x.status = Status::Working;
                x.blocked_reason = None;
                x.activity = activity;
            })
            .await?;
        Ok(())
    }

    /// True while a `request_repo` call for this agent is held and its client is still there. A client that hangs
    /// up is only noticed once the server drops its handler, a moment later.
    pub fn is_holding(&self, id: &AgentId) -> bool {
        self.waiters
            .lock()
            .expect("waiters poisoned")
            .get(&id.0)
            .is_some_and(|tx| !tx.is_closed())
    }

    /// Hands the answer to the held call; false when there is none any more (gone, timed out, never made).
    fn answer_held(&self, id: &AgentId, answer: RepoAnswer) -> bool {
        let tx = self.waiters.lock().expect("waiters poisoned").remove(&id.0);
        tx.is_some_and(|tx| tx.send(answer).is_ok())
    }

    /// A held call does not survive a daemon restart, so a request stored before it can never be answered.
    pub async fn clear_interrupted_requests(&self) -> Result<(), ServiceError> {
        let now = self.opts.clock.now();
        for a in self
            .store
            .snapshot()
            .await?
            .agents
            .into_iter()
            .filter(|a| a.repo_request.is_some())
        {
            let r = self
                .store
                .update_agent(&a.id, |x| {
                    if x.repo_request.take().is_none() || x.status == Status::Stopped {
                        return;
                    }
                    if x.status != Status::Working {
                        x.status_since = now;
                    }
                    x.status = Status::Working;
                    x.blocked_reason = None;
                    x.activity = "repo request interrupted by daemon restart".into();
                })
                .await;
            if let Err(e) = r {
                error!(agent_id = %a.id.0, error = %e, "cannot clear the interrupted repo request");
            }
        }
        Ok(())
    }

    /// Registers each repo the agent `git init`ed directly in its project folder as an owned workspace, so it
    /// shows on the card and in Diff, and so the archive knows never to remove it.
    pub async fn adopt_owned_repos(&self, a: &Agent) -> Result<(), ServiceError> {
        // A fleet agent's folder holds its one worktree; the solo CLAUDE.md written below would replace its own.
        if a.kind != AgentKind::Solo {
            return Ok(());
        }
        let workdir = PathBuf::from(&a.workdir);
        let entries = match std::fs::read_dir(&workdir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        let known = |ws: &[Workspace], p: &Path| ws.iter().any(|w| Path::new(&w.worktree) == p);
        let mut found: Vec<PathBuf> = entries
            .flatten()
            // file_type does not follow symlinks: a link to a repo elsewhere is not the agent's to own.
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.path())
            // A `.git` directory, not a worktree's `.git` file: only a real repo was made here.
            .filter(|p| std::fs::symlink_metadata(p.join(".git")).is_ok_and(|m| m.is_dir()))
            .filter(|p| !known(&a.workspaces, p))
            .collect();
        // Remembered under the project's persona, so a record none has adopted yet waits for its adoption.
        if found.is_empty() || a.persona.is_empty() {
            return Ok(());
        }
        found.sort();
        // An attach in progress holds this through a fetch; the next reconcile picks the repo up instead.
        let Ok(_ws) = self.workspaces_lock.try_lock() else {
            return Ok(());
        };
        let current = self.find(&a.id).await?;
        let mut added = Vec::new();
        for path in found.into_iter().filter(|p| !known(&current.workspaces, p)) {
            let row = match self.repos.remember_owned(&path, &a.persona).await {
                Ok(r) => r,
                Err(e) => {
                    warn!(agent_id = %a.id.0, path = %path.display(), error = %e, "cannot adopt repo");
                    continue;
                }
            };
            let branch = git::current_branch(&path)
                .await
                .ok()
                .flatten()
                .unwrap_or_else(|| "HEAD".into());
            added.push(Workspace {
                repo: row.name,
                branch,
                worktree: path.to_string_lossy().into_owned(),
                base: EMPTY_TREE.into(),
                from_branch: None,
                has_changes: false,
                owned: true,
            });
        }
        if added.is_empty() {
            return Ok(());
        }
        let pushed = added.clone();
        self.store
            .update_agent(&a.id, move |x| x.workspaces.extend(pushed))
            .await?;
        let mut all = current.workspaces;
        all.extend(added);
        // An invalid config leaves the file as it is: the agent made these repos, and the next spawn or attach says what is wrong.
        match fleet_config(&self.cfg.agent_config_dir) {
            Ok(fleet) => write_claude_md(&workdir, &fleet.instructions, &all)?,
            Err(e) => warn!(agent_id = %a.id.0, error = %e, "cannot rewrite CLAUDE.md"),
        }
        Ok(())
    }

    pub async fn answer(&self, id: &AgentId, text: &str) -> Result<(), ServiceError> {
        let a = self.running(id).await?;
        if text.trim().is_empty() {
            return Err(ServiceError::EmptyText);
        }
        self.tmux.send_literal(&a.tmux_session, text).await?;
        Ok(())
    }

    pub async fn pause(&self, id: &AgentId) -> Result<(), ServiceError> {
        let a = self.running(id).await?;
        if a.paused {
            return Ok(());
        }
        self.tmux.send_key(&a.tmux_session, "Escape").await?;
        self.store.update_agent(id, |a| a.paused = true).await?;
        Ok(())
    }

    pub async fn resume(&self, id: &AgentId) -> Result<(), ServiceError> {
        let a = self.running(id).await?;
        if !a.paused {
            return Ok(());
        }
        self.tmux.send_literal(&a.tmux_session, "continue").await?;
        self.store.update_agent(id, |a| a.paused = false).await?;
        Ok(())
    }

    /// Ends the session and keeps everything on disk: the worktrees, the branches and the project record. The pane
    /// is saved first, since a stopped agent has no terminal to show. Stopping a stopped agent changes nothing.
    pub async fn stop(&self, id: &AgentId) -> Result<(), ServiceError> {
        let a = self.find(id).await?;
        if a.status == Status::Stopped {
            // A stop whose kill failed left the session running, and reconcile and the orphan scan leave a recorded
            // session alone: a second Stop is the way out. Its bookkeeping was done by the first.
            if self.tmux.has_session(&a.tmux_session).await? {
                self.kill_or_gone(&a).await?;
                self.log_project(project_id(&a), "session ended on retry".into())
                    .await;
                info!(agent_id = %id.0, "stopped agent's session ended on retry");
            }
            return Ok(());
        }
        let live = self.tmux.has_session(&a.tmux_session).await?;
        // Best effort: a pane that cannot be read must not keep the agent running.
        let screen = if live {
            self.tmux.capture(&a.tmux_session).await.ok()
        } else {
            None
        };
        let now = self.opts.clock.now();
        // Written before the kill, so the death it causes (a SessionEnd hook, the reconcile timer) finds the agent
        // already Stopped and cannot name another cause. One write: the project record follows the agent only when
        // the status changes in the same call.
        self.store
            .update_agent(id, |x| {
                x.status = Status::Stopped;
                x.stop_cause = Some(StopCause::Stopped);
                x.activity = "stopped by you".into();
                x.status_since = now;
                x.paused = false;
            })
            .await?;
        let failed = if live {
            self.kill_or_gone(&a).await.err()
        } else {
            None
        };
        let project = project_id(&a);
        // The record's screen is its lead's: a specialist's would replace the one its card shows.
        if let Some(screen) = screen.filter(|_| a.kind != AgentKind::Specialist) {
            // The agent is stopped either way; a lost screen only leaves its terminal saying "not saved".
            if let Err(e) = self
                .store
                .update_project(project, |p| p.last_screen = Some(screen))
                .await
            {
                warn!(agent_id = %id.0, error = %e, "cannot save the stopped agent's screen");
            }
        }
        let line = match a.kind {
            AgentKind::Specialist => format!(
                "step {} ({}) stopped by you",
                a.step.unwrap_or(0),
                a.role.as_deref().unwrap_or("specialist")
            ),
            _ => "stopped by you".into(),
        };
        self.log_project(project, line).await;
        info!(agent_id = %id.0, "agent stopped");
        // Ends a held request_repo or propose_breakdown call now rather than when its hold runs out.
        self.waiters.lock().expect("waiters poisoned").remove(&id.0);
        self.breakdown_waiters
            .lock()
            .expect("waiters poisoned")
            .remove(&id.0);
        self.request_measure(project);
        // Reported only after the bookkeeping: the agent is Stopped either way, and Archive refuses while the
        // session lives (`SessionStillLive`).
        match failed {
            Some(e) => Err(e.into()),
            None => Ok(()),
        }
    }

    /// Kills the agent's session. A failed kill counts as done when the session is gone after all: it exited on
    /// its own between the stop's check and the kill.
    pub(crate) async fn kill_or_gone(&self, a: &Agent) -> Result<(), TmuxError> {
        let Err(e) = self.tmux.kill(&a.tmux_session).await else {
            return Ok(());
        };
        match self.tmux.has_session(&a.tmux_session).await {
            Ok(false) => Ok(()),
            _ => {
                error!(agent_id = %a.id.0, error = %e, "cannot end the stopped agent's session");
                Err(e)
            }
        }
    }

    async fn running(&self, id: &AgentId) -> Result<Agent, ServiceError> {
        let a = self.find(id).await?;
        if a.status == Status::Stopped || !self.tmux.has_session(&a.tmux_session).await? {
            return Err(ServiceError::NotRunning);
        }
        Ok(a)
    }

    /// Gives rows stored as `dead` (before `Stopped` carried a cause) a `stop_cause`, once at startup. A clean
    /// exit is told apart by the activity the old reducer wrote.
    pub async fn adopt_legacy_dead(&self) -> Result<(), ServiceError> {
        for a in self.store.snapshot().await?.agents {
            if a.status != Status::Stopped || a.stop_cause.is_some() {
                continue;
            }
            self.store
                .update_agent(&a.id, |x| {
                    if x.status != Status::Stopped || x.stop_cause.is_some() {
                        return;
                    }
                    if x.activity == "session ended" {
                        x.stop_cause = Some(StopCause::Exited);
                    } else {
                        x.stop_cause = Some(StopCause::Crashed);
                        if !x.activity.starts_with("crashed while ") {
                            x.activity = format!("crashed while {}", x.activity);
                        }
                    }
                })
                .await?;
        }
        Ok(())
    }

    /// Brings the store in line with reality. Safe to call repeatedly: at startup (to adopt agents that
    /// survived a daemon restart) and on a timer.
    #[tracing::instrument(skip_all)]
    pub async fn reconcile(&self) -> Result<(), ServiceError> {
        let agents = self.store.snapshot().await?.agents;
        self.reconcile_with(agents).await
    }

    /// Reconcile against an agent list read earlier. Every write re-checks the status it acts on, because a
    /// hook can land between the read and the write, and one agent failing never stops the others.
    pub async fn reconcile_with(&self, agents: Vec<Agent>) -> Result<(), ServiceError> {
        let live: HashSet<String> = self
            .tmux
            .list_sessions(SESSION_PREFIX)
            .await?
            .into_iter()
            .collect();
        let now = self.opts.clock.now();
        for a in agents {
            if a.status == Status::Stopped
                || self
                    .spawning
                    .lock()
                    .expect("spawning set poisoned")
                    .contains_key(&a.id.0)
            {
                continue;
            }
            if let Err(e) = self
                .reconcile_one(&a, live.contains(&a.tmux_session), now)
                .await
            {
                error!(agent_id = %a.id.0, error = %e, "reconcile failed");
            }
        }
        Ok(())
    }

    async fn reconcile_one(
        &self,
        a: &Agent,
        session_alive: bool,
        now: i64,
    ) -> Result<(), ServiceError> {
        if session_alive {
            // Separate from the status rules below, so a repo that cannot be adopted never holds them up.
            if let Err(e) = self.adopt_owned_repos(a).await {
                error!(agent_id = %a.id.0, error = %e, "adopt failed");
            }
        }
        if !session_alive {
            let mut died = false;
            self.store
                .update_agent(&a.id, |x| {
                    if x.status != Status::Stopped {
                        died = true;
                        x.status = Status::Stopped;
                        x.stop_cause = Some(StopCause::Crashed);
                        x.activity = format!("crashed while {}", x.activity);
                        x.blocked_reason = None;
                        x.repo_request = None;
                        x.paused = false;
                        x.status_since = now;
                    }
                })
                .await?;
            if died {
                info!(agent_id = %a.id.0, "agent session lost");
                self.request_measure(&a.id);
            }
            self.waiters
                .lock()
                .expect("waiters poisoned")
                .remove(&a.id.0);
            self.breakdown_waiters
                .lock()
                .expect("waiters poisoned")
                .remove(&a.id.0);
        } else if a.status == Status::Queued && now - a.status_since >= self.opts.startup_grace_secs
        {
            // No hook fires before Claude's first-run "trust this folder" dialog; silence is the signal.
            self.store
                .update_agent(&a.id, |x| {
                    if x.status == Status::Queued {
                        x.status = Status::NeedsYou;
                        x.activity = "waiting at startup prompt".into();
                        x.blocked_reason = Some(BlockedReason::Startup);
                        x.status_since = now;
                    }
                })
                .await?;
        } else if a.status == Status::Review {
            let mut flags = Vec::with_capacity(a.workspaces.len());
            for w in &a.workspaces {
                let path = std::path::Path::new(&w.worktree);
                let changed = if w.owned {
                    git::owned_has_changes(path).await
                } else {
                    git::has_changes(path).await
                };
                flags.push(changed.unwrap_or(w.has_changes));
            }
            if flags
                .iter()
                .zip(&a.workspaces)
                .any(|(f, w)| *f != w.has_changes)
            {
                self.store
                    .update_agent(&a.id, |x| {
                        for (w, f) in x.workspaces.iter_mut().zip(&flags) {
                            w.has_changes = *f;
                        }
                        x.has_changes = x.workspaces.iter().any(|w| w.has_changes);
                    })
                    .await?;
            }
        }
        Ok(())
    }

    /// Hooks are fire-and-forget, so events sent while the daemon was down are gone. Agents that were
    /// working or blocked may be in a different state than the store says; say so until the next hook
    /// replaces the doubt with fact. A finished (Review) agent has no reason to have changed.
    pub async fn mark_unverified(&self) -> Result<(), ServiceError> {
        let live: HashSet<String> = self
            .tmux
            .list_sessions(SESSION_PREFIX)
            .await?
            .into_iter()
            .collect();
        for a in self.store.snapshot().await?.agents {
            if live.contains(&a.tmux_session)
                && matches!(
                    a.status,
                    Status::Queued | Status::Working | Status::NeedsYou
                )
            {
                let r = self
                    .store
                    .update_agent(&a.id, |x| {
                        if matches!(
                            x.status,
                            Status::Queued | Status::Working | Status::NeedsYou
                        ) {
                            x.activity = "status unverified since daemon restart".into();
                        }
                    })
                    .await;
                match r {
                    Ok(_) => {
                        self.log_project(&a.id, "daemon restarted, status unverified".into())
                            .await
                    }
                    Err(e) => {
                        error!(agent_id = %a.id.0, error = %e, "cannot mark the agent unverified")
                    }
                }
            }
        }
        Ok(())
    }

    /// What each workspace changed since its branch started (committed, uncommitted and untracked).
    pub async fn diff(&self, id: &AgentId) -> Result<DiffReport, ServiceError> {
        let a = self.find(id).await?;
        let mut workspaces = Vec::with_capacity(a.workspaces.len());
        for w in &a.workspaces {
            let d = git::diff(std::path::Path::new(&w.worktree), &w.base, MAX_DIFF_BYTES).await?;
            workspaces.push(WorkspaceDiff {
                repo: w.repo.clone(),
                diff: d.text,
                truncated: d.truncated,
            });
        }
        Ok(DiffReport { workspaces })
    }
}

#[cfg(test)]
mod tests {
    use super::one_line;

    #[test]
    fn one_line_turns_control_characters_and_newlines_into_single_spaces() {
        assert_eq!(one_line("a\nb\r\nc"), "a b c");
        // Escape would start a key sequence in the agent's terminal; Ctrl-C (\x03) would interrupt it.
        assert_eq!(one_line("x\x1b[2J\x03y\tz"), "x [2J y z");
        assert_eq!(one_line("  \u{7f}  "), "");
        assert_eq!(one_line("fine: été"), "fine: été");
    }
}
