//! The agent lifecycle: spawn, answer, pause, resume, stop, and reconcile with the sessions that really run.
use crate::agent_config;
use crate::config::Config;
use crate::fleet::breakdown::{BreakdownAnswer, BreakdownError};
use crate::fleet::defs::FleetError;
use crate::fleet::ops::{agent_vars, INTERNAL_DIR, ORCHESTRATOR_DIR};
use crate::fleet::workflow::{self, WorkflowError, WORKFLOW_FILE};
use crate::footprint;
use crate::git::{self, GitError, Worktree};
use crate::hooks_cfg::{self, write_mcp_config, write_settings, RoleSettings, SettingsError};
use crate::internal::{Internal, PushOutcome};
use crate::orphans;
use crate::personas::{ambient_vars, PersonaEnv, PersonaError, Personas};
use crate::repo_rules::RepoRules;
use crate::repos::{RepoError, Repos, Resolved, Start as RepoStart};
use crate::session::{nz, Key, Output, SessionError, SessionHost, Spec, DEFAULT_SIZE};
use crate::store::{RepoRow, Store, StoreError};
use fleet_api::{
    task_id, Agent, AgentConfig, AgentId, AgentKind, ArchiveReport, BlockedReason, DiffReport,
    GateRun, LogEntry, Orphan, OrphanKind, RepoRequest, SettingSource, SpawnRequest, SpawnResponse,
    Status, StopCause, TaskState, ToolId, Workspace, WorkspaceDiff,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{debug, error, info, warn};

pub const SESSION_PREFIX: &str = "fleet-";
const MAX_PROMPT_BYTES: usize = 20_000;
/// The prompt's file in the task folder. Not on `claude`'s command line, which any local user can read in `ps`.
pub const PROMPT_FILE: &str = "PROMPT.md";
/// What the command line says instead of the prompt.
pub const PROMPT_POINTER: &str =
    "Your task is in PROMPT.md in this folder. Read it first, then do it.";
const MAX_DIFF_BYTES: usize = 1 << 20;
/// git's empty tree: the base of a repo the agent made itself, so its diff shows everything in it.
pub const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// Set on every agent's process so Claude Code keeps a `request_repo` call open while the owner decides. With
/// its defaults it gives up after 60 s, or moves the call to the background at 120 s and drops it after 300 s
/// of silence (measured in a spike).
pub const MCP_HOLD_ENV: [(&str, &str); 3] = [
    ("MCP_TOOL_TIMEOUT", "2400000"),
    ("CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT", "0"),
    ("CLAUDE_CODE_MCP_AUTO_BACKGROUND_MS", "3600000"),
];

/// The card's activity for an agent relaunched by `resume_interrupted`.
pub const RESUMED: &str = "resumed after daemon restart";
/// Claude's prompt for an agent interrupted mid-work.
pub const RESUME_PROMPT: &str =
    "The fleet daemon restarted and interrupted you. Continue where you left off.";
/// A resumed claude that exits within this long could not resume; it is stopped, not retried.
const RESUME_GRACE: Duration = Duration::from_secs(5);

/// How `launch` starts claude: a new conversation, or the agent's own one again after a daemon restart.
pub(crate) enum Start {
    New,
    Resume {
        claude_session: String,
        prompt: Option<&'static str>,
    },
}

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
    /// Program and leading args used to start Claude Code, e.g. `["claude"]`. The program runs from where `tools`
    /// found it, so `tools` is built from the same command.
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
    /// Test seam: `propose_breakdown` waits on it between reading the breakdown and storing the new version, and
    /// `finish_step` between stopping the agent and marking its step done, so a test can act in either gap. Always
    /// `None` in the daemon.
    #[doc(hidden)]
    pub fleet_gate: Option<Arc<RequestGate>>,
    /// Test seam: each orphan scan waits on it once it has read the disk, before it writes the cache, so a test can
    /// act while a scan runs. Always `None` in the daemon.
    #[doc(hidden)]
    pub scan_gate: Option<Arc<RequestGate>>,
    /// How long one `gh pr list` of the merge watcher may take; tests shorten it.
    pub merge_check_timeout: Duration,
    /// Test seam: added to every claude process's environment. Always empty in the daemon.
    #[doc(hidden)]
    pub extra_env: Vec<(String, String)>,
    /// The tools on this machine; every session's PATH is the one their last check found.
    pub tools: crate::tools::Tools,
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
            fleet_gate: None,
            scan_gate: None,
            merge_check_timeout: crate::merges::MERGE_CHECK_TIMEOUT,
            extra_env: Vec::new(),
            // The daemon's own PATH as is, and the config's defaults; `main` builds the real one.
            tools: crate::tools::Tools::new(
                fleet_api::Os::current(),
                Arc::new(crate::tools::CurrentPath),
                crate::config::home_dir(),
                "gh".into(),
                vec!["claude".into()],
            ),
        }
    }
}

/// The running count as a refusal states it.
fn running_words(n: usize) -> String {
    match n {
        1 => "1 specialist is running".into(),
        n => format!("{n} specialists are running"),
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
    Internal(#[from] crate::internal::InternalError),
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("cannot write the task directory: {0}")]
    Io(#[from] std::io::Error),
    /// A tool the agent needs (claude) is missing or does not work.
    #[error(transparent)]
    Tool(Box<fleet_api::ToolError>),
    #[error(transparent)]
    Persona(#[from] PersonaError),
    #[error("{NO_PERSONA}")]
    NoPersona,
    #[error("the fleet-wide agent config is invalid: {0}")]
    AgentConfig(String),
    #[error("{ONE_REPO}")]
    OneRepo,
    /// The orchestrated task's repo has no usable workflow on its base, or the base could not be fetched.
    #[error("{}", workflow_refusal(repo, error))]
    Workflow { repo: String, error: WorkflowError },
    #[error(transparent)]
    Fleet(#[from] FleetError),
    #[error("a specialist is started by its orchestrator, not on its own")]
    SpecialistSpawn,
    #[error("pick at least one specialist for this task")]
    NoRoster,
    #[error("release {release} is not a release of {product}")]
    ReleaseNotInProduct { release: i64, product: String },
    #[error("release {0} is already released")]
    Released(String),
}

impl From<fleet_api::ToolError> for SpawnError {
    fn from(e: fleet_api::ToolError) -> Self {
        SpawnError::Tool(Box::new(e))
    }
}

impl SpawnError {
    /// The tool behind the refusal, when a missing or unusable one is what stopped it.
    pub fn tool(&self) -> Option<&fleet_api::ToolError> {
        match self {
            SpawnError::Tool(t) | SpawnError::Persona(PersonaError::Tool(t)) => Some(t),
            _ => None,
        }
    }
}

const ONE_REPO: &str = "an orchestrated task has exactly one repo";
const NO_PERSONA: &str = "pick a persona for this task";

fn workflow_refusal(repo: &str, error: &WorkflowError) -> String {
    match error {
        WorkflowError::Missing { base } => format!(
            "{repo} has no {WORKFLOW_FILE} on {base}. It lists the checks the gate runs before a pull request, so \
             orchestrated tasks need it."
        ),
        other => format!("{repo}: {other}"),
    }
}

/// How a limit's source reads in a refusal the orchestrator relays to the person.
fn source_word(s: SettingSource) -> &'static str {
    match s {
        SettingSource::Product => "product",
        SettingSource::Global => "global",
        SettingSource::Definition | SettingSource::Default => "default",
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("no such agent")]
    NotFound,
    #[error("agent is not running")]
    NotRunning,
    #[error("no such orphan")]
    NotAnOrphan,
    #[error("the task is not stopped; stop it first")]
    NotStopped,
    /// A stopped agent whose session still runs: its claude would still run in the worktrees an archive deletes.
    #[error("the agent's session is still running; end it before archiving")]
    SessionStillLive,
    #[error("{}", fleet_api::PLAN_CHANGED)]
    PlanChanged,
    /// Something an archive could not remove (an I/O failure); the text names it.
    #[error("{0}")]
    Removal(String),
    /// Something an archive deliberately would not touch (e.g. a symbolic link); the text says why.
    #[error("{0}")]
    Refused(String),
    /// An archive refused while the task's PR (its URL) is open: archiving would drop a task whose PR may still merge.
    #[error("{0} is still open; merge or close it before archiving")]
    OpenPr(String),
    #[error("text is empty")]
    EmptyText,
    #[error(transparent)]
    Repo(#[from] RepoError),
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Session(#[from] SessionError),
    /// A specialist or restarted orchestrator whose `claude` did not start.
    #[error(transparent)]
    Launch(#[from] SpawnError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("cannot write the task directory: {0}")]
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
    #[error("no answer yet; call wait_for_fleet: the decision arrives there when it is made")]
    BreakdownNoAnswerYet,
    /// A loop guard: the orchestrator stops and asks the person, who can raise the limit for the product.
    #[error("limit reached: {key} = {value} ({}), counted over the whole task. Stop and ask the person; they can raise it for this product in Management → Products.", source_word(*source))]
    LimitReached {
        key: &'static str,
        value: u32,
        // Raw identifier: thiserror takes a field named `source` for the error's cause, and this is not one.
        r#source: SettingSource,
    },
    /// The parallel cap: not a stop, the orchestrator waits for a specialist to finish and spawns again.
    #[error("{} (max_parallel_specialists = {value}, {}). Call wait_for_fleet and spawn this step when one finishes.", running_words(*running), source_word(*source))]
    TooManyRunning {
        /// How many specialists run now; above `value` when the limit was lowered under them.
        running: usize,
        value: u32,
        r#source: SettingSource,
    },
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
    /// The repo's tests failed or timed out, so nothing was pushed; the run is on the task record too.
    #[error("{}", gate_failure_text(.0))]
    GateFailed(GateRun),
    /// A tool the call needs (Git Bash for the gate's steps) is missing or does not work.
    #[error(transparent)]
    Tool(Box<fleet_api::ToolError>),
}

impl ServiceError {
    /// The tool behind the refusal, when a missing or unusable one is what stopped it.
    pub fn tool(&self) -> Option<&fleet_api::ToolError> {
        match self {
            ServiceError::Launch(e) => e.tool(),
            ServiceError::Persona(PersonaError::Tool(t)) | ServiceError::Tool(t) => Some(t),
            _ => None,
        }
    }
}

/// A failed `insert_agent_under_persona`: a persona deleted mid-spawn answers as one that was never there (404).
pub(crate) fn agent_insert_error<E: From<PersonaError> + From<StoreError>>(e: StoreError) -> E {
    match e {
        StoreError::PersonaGone(p) => PersonaError::NotFound(p).into(),
        e => e.into(),
    }
}

/// How a failed gate reads to the orchestrator, which answers it by proposing a fix step: the failed step, what it
/// printed, and why every step ran if a file nothing covers made it.
pub fn gate_failure_text(run: &GateRun) -> String {
    let passed: Vec<&str> = run
        .steps
        .iter()
        .filter(|s| s.exit_code == Some(0))
        .map(|s| s.name.as_str())
        .collect();
    let mut text = match run.steps.iter().find(|s| s.exit_code != Some(0)) {
        Some(s) => {
            let why = match s.exit_code {
                Some(code) => format!("exit {code}"),
                None => format!("timed out after {}s", s.duration_secs),
            };
            let mut first = format!("tests failed in step {} ({why})", s.name);
            if !passed.is_empty() {
                first.push_str(&format!("; passed: {}", passed.join(", ")));
            }
            format!("{first}\n{}", s.output_tail)
        }
        None => "tests failed: no step ran".to_string(),
    };
    if !run.unmatched.is_empty() {
        text.push_str(&format!(
            "\nran every step: no step covers {}",
            run.unmatched.join(", ")
        ));
    }
    text
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
    /// False when the task already had it.
    new: bool,
}

/// The deny and ask rules of every repo in `workspaces`, rewritten for the task folder (see `repo_rules`).
pub fn task_repo_rules(workspaces: &[Workspace]) -> RepoRules {
    let repos: Vec<(String, PathBuf)> = workspaces
        .iter()
        .map(|w| (folder(w), PathBuf::from(&w.worktree)))
        .collect();
    RepoRules::collect(repos.iter().map(|(f, p)| (f.as_str(), p.as_path())))
}

const START_A_REPO: &str = "To start a new repo, create a subfolder and run `git init` in it.";
const ASK_FOR_A_REPO: &str =
    "To work in an existing repo, call the `request_repo` tool (fleet MCP server); never clone or copy repos yourself.";

/// The subfolder a workspace lives in, relative to the task folder.
fn folder(w: &Workspace) -> String {
    Path::new(&w.worktree)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| w.repo.clone())
}

/// How the daemon's `CLAUDE.md` starts: an orphan folder whose `CLAUDE.md` does not is not taken for the daemon's.
pub const CLAUDE_MD_HEADER: &str =
    "# Fleet Manager task\n\nThis folder is a task folder, not a git repository.";

/// `CLAUDE.md` for a task folder: the fleet-wide instructions, where the repos are, how to get more, plus an
/// import of each repo's own `CLAUDE.md`.
fn task_claude_md(instructions: &str, workspaces: &[Workspace]) -> String {
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

/// Where the repos are, and with `more_repos` how to get others (an orchestrated task has its one), plus an
/// import of each repo's own `CLAUDE.md`.
pub(crate) fn repo_section(workspaces: &[Workspace], more_repos: bool) -> String {
    let mut text = String::from(CLAUDE_MD_HEADER);
    if workspaces.is_empty() {
        text.push_str(" This task has no repos yet.\n");
    } else {
        text.push_str(" Each repo of this task is a subfolder:\n\n");
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

/// The one command that commits everything in the notes checkout at `path`. A plain `git -C <path> commit` commits
/// nothing new: a new file is untracked until it is added.
pub(crate) fn internal_commit_command(path: &Path) -> String {
    let path = path.display();
    format!("git -C {path} add -A && git -C {path} commit -m \"<message>\"")
}

/// Which copy of a product's private notes an agent gets.
pub(crate) enum InternalCopy<'a> {
    /// The orchestrator's own writable worktree, on the branch of the task named here.
    Writable { task: &'a str },
    /// A specialist's read-only, detached copy at this commit.
    Snapshot { sha: &'a str },
}

/// Where a product's private notes are for this agent.
pub(crate) fn internal_section(path: &Path, copy: InternalCopy<'_>) -> String {
    let commit = internal_commit_command(path);
    let path = path.display();
    match copy {
        // The task's folder is spelled out: the skills and the orchestrator prompt say `tasks/<task-name>/`, and
        // an agent left to guess the slug writes somewhere else.
        InternalCopy::Writable { task } => format!(
            "## Internal resources\n\n\
`{path}` is this product's private notes: designs, plans and decisions, kept by the app in its own git repo,\n\
the product's shared internal repo, which is pushed when the task opens its pull request. This task's folder there is `tasks/{task}/` (the `tasks/<task-name>/` of your instructions and\n\
skills): write its design and plan in it, and commit them with\n\
`{commit}`.\n\
A specialist reads these notes in its own read-only copy, so a brief names a note by its path relative to this\n\
folder, such as `tasks/{task}/plan.md`, never by its path here.\n\
Never copy anything from here into a repo or a pull request.\n"
        ),
        InternalCopy::Snapshot { sha } => format!(
            "## Internal resources\n\n\
`{path}` is a read-only copy of this task's private notes (the design, the plan, decisions) as of commit\n\
`{sha}`. Read what your brief points you to here. Do not edit it, and never copy anything from it into a repo or\n\
a pull request.\n"
        ),
    }
}

/// Readable by the owner only: the prompt can hold anything the user typed.
pub(crate) fn write_prompt(workdir: &Path, prompt: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    // Owner-only on Unix; on Windows the task folder sits in the user's profile, whose ACL is already owner-only.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut f = options.open(workdir.join(PROMPT_FILE))?;
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
        task_claude_md(instructions, workspaces),
    )
}

/// The fleet-wide config as it is on disk now: read on every spawn and attach so an edit applies to the next one.
pub(crate) fn fleet_config(dir: &Path) -> Result<AgentConfig, String> {
    agent_config::load_valid(dir).map_err(|e| e.to_string())
}

/// What spawn must remove again if a later step fails.
pub(crate) struct Undo {
    /// Each worktree made so far, with the base it hangs off.
    pub made: Vec<(PathBuf, Worktree)>,
    pub workdir: PathBuf,
    pub settings_dir: PathBuf,
    /// An orchestrator's internal worktree: the product and task name whose branch to delete with it, since a task
    /// that never existed has no notes to keep.
    pub internal: Option<(String, String)>,
}

/// A newline would press Enter and submit half a line.
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
    /// Ids queued and not yet picked up: a second request for the same task adds nothing.
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
                MeasureJob::All => match self.store.tasks().await {
                    Ok(all) => (
                        all.into_iter()
                            .filter(|p| p.state != TaskState::Archived)
                            .map(|p| p.id)
                            .collect(),
                        true,
                    ),
                    Err(e) => {
                        warn!(error = %e, "cannot list tasks to measure");
                        self.all_pending.store(false, Ordering::SeqCst);
                        continue;
                    }
                },
            };
            for id in ids {
                if let Err(e) = self.measure_one(&id).await {
                    warn!(agent_id = %id.0, error = %e, "cannot measure task");
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
        let Some(p) = self.store.task(id).await? else {
            return Ok(());
        };
        let workdir = PathBuf::from(&p.workdir);
        // No folder, nothing to weigh: keep the last number rather than record zero.
        if p.state == TaskState::Archived || !workdir.is_dir() {
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
            .update_task(id, |p| {
                if p.state != TaskState::Archived {
                    p.footprint = Some(fp)
                }
            })
            .await?;
        match self
            .store
            .update_agent(id, |a| a.disk_bytes = Some(total))
            .await
        {
            // A task whose agent record is gone (archived earlier) has only the footprint to update.
            Ok(_) | Err(StoreError::NotFound(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }
}

/// Queues one task's measurement unless it is already queued.
fn queue_measure(
    queued: &Mutex<HashSet<String>>,
    tx: &mpsc::UnboundedSender<MeasureJob>,
    id: &AgentId,
) {
    if queued.lock().expect("queued poisoned").insert(id.0.clone()) {
        let _ = tx.send(MeasureJob::One(id.clone()));
    }
}

/// What stopping an agent a restart could not resume needs, so the early-exit watcher never holds the service.
#[derive(Clone)]
struct Unresumable {
    store: Arc<Store>,
    clock: Arc<dyn Clock>,
    measure_tx: mpsc::UnboundedSender<MeasureJob>,
    measure_queued: Arc<Mutex<HashSet<String>>>,
    resuming: Arc<Mutex<HashSet<String>>>,
    shutting_down: Arc<AtomicBool>,
}

impl Unresumable {
    /// Stops the agent with cause `Interrupted`; an empty `why` means there was nothing to resume. An agent already
    /// stopped (by its owner, meanwhile) keeps its own cause.
    async fn stop(&self, id: &AgentId, why: &str) -> Result<(), StoreError> {
        let now = self.clock.now();
        let activity = if why.is_empty() {
            "stopped by a daemon restart".to_string()
        } else {
            format!("could not resume: {}", one_line(why))
        };
        let mut stopped = false;
        self.store
            .update_agent(id, |x| {
                if x.status == Status::Stopped {
                    return;
                }
                stopped = true;
                x.status = Status::Stopped;
                x.stop_cause = Some(StopCause::Interrupted);
                x.activity = activity;
                x.blocked_reason = None;
                x.repo_request = None;
                x.paused = false;
                x.status_since = now;
            })
            .await?;
        if stopped {
            info!(agent_id = %id.0, "agent could not be resumed");
            queue_measure(&self.measure_queued, &self.measure_tx, id);
        }
        Ok(())
    }

    /// Waits up to `RESUME_GRACE` for the resumed session to exit, drawing its screen meanwhile; an exit within it
    /// stops the agent with the screen's last line. Reconcile leaves the agent alone until this is done.
    async fn watch(self, id: AgentId, mut rx: broadcast::Receiver<Output>) {
        let mut screen = vt100::Parser::new(nz(DEFAULT_SIZE.rows), nz(DEFAULT_SIZE.cols), 0);
        let exited = tokio::time::timeout(RESUME_GRACE, async {
            loop {
                match rx.recv().await {
                    Ok(Output::Bytes(b)) => screen.process(&b),
                    Ok(Output::Exited(_)) | Err(broadcast::error::RecvError::Closed) => {
                        return true
                    }
                    // Only the last line is wanted, and output after the gap still draws it.
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                }
            }
        })
        .await
        .unwrap_or(false);
        // A session the daemon's own shutdown killed is resumed again at the next start.
        if exited && !self.shutting_down.load(Ordering::SeqCst) {
            let text = screen.screen().contents();
            let last = text
                .lines()
                .map(str::trim)
                .rfind(|l| !l.is_empty())
                .unwrap_or("claude exited at once");
            if let Err(e) = self.stop(&id, last).await {
                error!(agent_id = %id.0, error = %e, "cannot stop the agent that could not resume");
            }
        }
        self.resuming
            .lock()
            .expect("resuming poisoned")
            .remove(&id.0);
    }
}

/// What the task records, the agents and the spawns in flight own: what is never an orphan.
#[derive(Default)]
pub(crate) struct Owned {
    pub(crate) workdirs: HashSet<PathBuf>,
    pub(crate) worktrees: Vec<PathBuf>,
    pub(crate) ids: HashSet<String>,
}

impl Owned {
    /// A folder or worktree is owned when it holds, or sits in, anything a record names.
    pub(crate) fn owns(&self, o: &Orphan) -> bool {
        let path = Path::new(&o.path);
        match o.kind {
            OrphanKind::TaskFolder | OrphanKind::Worktree => self
                .workdirs
                .iter()
                .chain(&self.worktrees)
                .any(|d| d.starts_with(path) || path.starts_with(d)),
            OrphanKind::AgentFolder => path
                .file_name()
                .is_some_and(|n| self.ids.contains(n.to_string_lossy().as_ref())),
        }
    }
}

pub struct AgentService {
    pub(crate) cfg: Config,
    personas: Arc<Personas>,
    pub(crate) store: Arc<Store>,
    pub(crate) repos: Arc<Repos>,
    /// The one handle on the internal repos: its per-product locks only serialise git if every caller shares it.
    pub(crate) internal: Arc<Internal>,
    pub(crate) sessions: SessionHost,
    pub(crate) opts: ServiceOptions,
    /// Agents mid-spawn, by id, with their task folder: from before the folder exists until the session does.
    /// Reconcile must not call them lost, and the orphan scan must not call their folder an orphan.
    spawning: Mutex<HashMap<String, PathBuf>>,
    /// Held from choosing a task directory until it exists, so two spawns never pick the same name. An orphan
    /// archive holds it while it re-checks and removes, so no new folder appears under it meanwhile.
    pub(crate) pick_lock: tokio::sync::Mutex<()>,
    /// Held while a task's workspace list is read, extended and written back (attach, adopt, archive), so two
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
    /// Tasks whose `open_pr` is past its breakdown check, by task id. A proposal or spawn for one is refused
    /// rather than held: the gate can run for half an hour, and `breakdown_lock` is every task's.
    pub(crate) gating: Mutex<HashSet<String>>,
    /// The queue of the one task that measures task folders, started by the first request (`new` may run
    /// outside a runtime).
    measure_tx: OnceLock<mpsc::UnboundedSender<MeasureJob>>,
    measure_queued: Arc<Mutex<HashSet<String>>>,
    measure_all_pending: Arc<AtomicBool>,
    measure_all_passes: Arc<AtomicUsize>,
    clones_bytes: Arc<Mutex<Option<u64>>>,
    /// True from a requested rescan until it starts, so a refresh meanwhile adds nothing; one made while a scan
    /// runs queues the next, since the running one may have read the disk before the change it was asked for.
    rescan_pending: Arc<AtomicBool>,
    rescans: AtomicUsize,
    /// Held through each scan and its write to the cache, so a slower, older scan never overwrites a newer one.
    rescan_lock: tokio::sync::Mutex<()>,
    /// What the last scan found, with sizes filled in as the background walk reaches them.
    orphans: Arc<Mutex<Vec<Orphan>>>,
    /// Bumped by each scan's write, so a sizing walk a newer scan has replaced stops instead of walking twice.
    orphans_gen: Arc<AtomicU64>,
    /// The merge watcher's queue, started by the first request (`new` may run outside a runtime).
    pub(crate) merge_queue: OnceLock<crate::merges::MergeQueue>,
    pub(crate) merge_runs: Arc<AtomicUsize>,
    /// Agents relaunched by `resume_interrupted` whose grace is still running: an early exit is the watcher's to
    /// record, so reconcile must not call it a crash first.
    resuming: Arc<Mutex<HashSet<String>>>,
    /// Set by `begin_shutdown`: the sessions are about to be killed, and nothing may record their end.
    shutting_down: Arc<AtomicBool>,
}

pub(crate) struct SpawningGuard<'a>(&'a Mutex<HashMap<String, PathBuf>>, String);

/// Clears a flag when dropped, so a task that panics or is cancelled cannot leave it set.
struct ClearOnDrop<'a>(&'a AtomicBool);

impl Drop for ClearOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

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
        sessions: SessionHost,
        opts: ServiceOptions,
    ) -> AgentService {
        let personas = Arc::new(Personas::new(
            store.clone(),
            cfg.personas_dir.clone(),
            cfg.claude_command.clone(),
            cfg.gh_command.clone(),
            cfg.tasks_root.clone(),
            cfg.state_dir.clone(),
            opts.tools.clone(),
        ));
        let internal = Arc::new(Internal::new(cfg.state_dir.join(INTERNAL_DIR)));
        AgentService {
            cfg,
            personas,
            store,
            repos,
            internal,
            sessions,
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
            rescan_lock: tokio::sync::Mutex::new(()),
            orphans: Arc::new(Mutex::new(Vec::new())),
            orphans_gen: Arc::new(AtomicU64::new(0)),
            merge_queue: OnceLock::new(),
            merge_runs: Arc::new(AtomicUsize::new(0)),
            resuming: Arc::new(Mutex::new(HashSet::new())),
            shutting_down: Arc::new(AtomicBool::new(false)),
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

    pub fn internal(&self) -> &Arc<Internal> {
        &self.internal
    }

    pub fn products(&self) -> crate::products::Products {
        crate::products::Products::new(
            self.store.clone(),
            self.repos.clone(),
            self.opts.clock.clone(),
            self.internal.clone(),
            self.personas.clone(),
        )
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

    /// Queues a disk measurement of one task and returns at once; the result lands on its record and card.
    /// Must be called inside the tokio runtime.
    pub fn request_measure(&self, id: &AgentId) {
        queue_measure(&self.measure_queued, self.measure_queue(), id);
    }

    /// Must be called inside the tokio runtime (it may start the measuring task).
    fn unresumable(&self) -> Unresumable {
        Unresumable {
            store: self.store.clone(),
            clock: self.opts.clock.clone(),
            measure_tx: self.measure_queue().clone(),
            measure_queued: self.measure_queued.clone(),
            resuming: self.resuming.clone(),
            shutting_down: self.shutting_down.clone(),
        }
    }

    /// Called before the daemon kills every session on its way out: those ends are its own doing, so reconcile and
    /// the resume watchers leave the agents as they are, for the next start to resume.
    pub fn begin_shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
    }

    /// Queues a measurement of every task that is not archived, and of the clone cache. A request made while a
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

    /// `rescan_orphans` in the background, unless one is already queued; after the running one, if any.
    /// Must be called inside the tokio runtime.
    pub fn request_rescan(self: &Arc<Self>) {
        if self.rescan_pending.swap(true, Ordering::SeqCst) {
            return;
        }
        let svc = self.clone();
        tokio::spawn(async move {
            let queued = ClearOnDrop(&svc.rescan_pending);
            let _scan = svc.rescan_lock.lock().await;
            drop(queued);
            svc.rescan_locked().await;
            svc.rescans.fetch_add(1, Ordering::SeqCst);
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
    /// disk before this call is owned by one or the other.
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
            owned.ids.insert(id);
            owned.workdirs.insert(workdir);
        }
        for p in self.store.tasks().await? {
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
            owned.ids.insert(a.id.0);
        }
        Ok(owned)
    }

    /// Folders only: a session never lacks an agent, since sessions end with the daemon.
    async fn find_orphans(&self) -> Result<Vec<Orphan>, ServiceError> {
        let bases = self.repos.all_bases().await?;
        let before = self.owned_now().await?;
        let root = self.cfg.tasks_root.clone();
        let agents_dir = self.cfg.state_dir.join("agents");
        let mut found = tokio::task::spawn_blocking(move || {
            orphans::scan(&root, &agents_dir, &bases, &before.workdirs, &before.ids)
        })
        .await
        .map_err(std::io::Error::other)?;
        // A spawn can make its folder, and store its record, while the scan walks: checked again once it is done.
        let after = self.owned_now().await?;
        found.retain(|o| !after.owns(o));
        if let Some(gate) = &self.opts.scan_gate {
            gate.reached.notify_one();
            gate.open.notified().await;
        }
        Ok(found)
    }

    /// Scans again and replaces the cached list. Sizes are walked afterwards in the background (one folder at a
    /// time, off the async threads), so this returns after the directory names and a few `git` calls.
    /// Must be called inside the tokio runtime.
    pub async fn rescan_orphans(&self) {
        let _scan = self.rescan_lock.lock().await;
        self.rescan_locked().await;
    }

    async fn rescan_locked(&self) {
        let found = match self.find_orphans().await {
            Ok(f) => f,
            Err(e) => {
                warn!(error = %e, "cannot scan for orphans");
                return;
            }
        };
        let to_weigh: Vec<(String, PathBuf)> = found
            .iter()
            .map(|o| (o.id.clone(), PathBuf::from(&o.path)))
            .collect();
        if !found.is_empty() {
            info!(count = found.len(), "orphans found");
        }
        *self.orphans.lock().expect("orphans poisoned") = found;
        let gen = self.orphans_gen.fetch_add(1, Ordering::SeqCst) + 1;
        let gens = self.orphans_gen.clone();
        let cache = self.orphans.clone();
        tokio::spawn(async move {
            for (id, path) in to_weigh {
                if gens.load(Ordering::SeqCst) != gen {
                    break;
                }
                // A registered worktree whose folder is gone has no size; 0 would read as an empty folder.
                let weigh = move || {
                    std::fs::symlink_metadata(&path)
                        .is_ok()
                        .then(|| footprint::dir_bytes(&path))
                };
                let Ok(Some(bytes)) = tokio::task::spawn_blocking(weigh).await else {
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

    /// What pickers and `list_repos` offer, all or one product's: repos owned by a task are left out (see
    /// `Repos::pickable`).
    pub async fn remembered_repos(
        &self,
        product: Option<&str>,
    ) -> Result<Vec<RepoRow>, ServiceError> {
        Ok(self.repos.pickable(product).await?)
    }

    pub(crate) async fn find(&self, id: &AgentId) -> Result<Agent, ServiceError> {
        let snap = self.store.snapshot().await?;
        snap.agents
            .into_iter()
            .find(|a| &a.id == id)
            .ok_or(ServiceError::NotFound)
    }

    /// First free `<slug>`, `<slug>-2`, ...: no such task folder and no `<prefix>/<name>` branch in any repo's
    /// checkout or clone, since stopping and archiving keep branches and any of those repos may be attached to this
    /// task later.
    async fn free_task_name(&self, product: &str, slug: &str) -> Result<String, SpawnError> {
        let bases = self.repos.all_bases().await?;
        for n in 1.. {
            let name = if n == 1 {
                slug.to_string()
            } else {
                format!("{slug}-{n}")
            };
            if self.cfg.tasks_root.join(&name).exists() {
                continue;
            }
            let branch = format!("{}/{name}", self.cfg.branch_prefix);
            let mut taken = false;
            for base in &bases {
                if git::branch_exists(base, &branch).await? {
                    taken = true;
                    break;
                }
            }
            // Archiving keeps the notes branch, so a name an old task used is not free for a new one's notes.
            if !taken
                && !self
                    .internal
                    .branch_exists(product, &Internal::task_branch(&name))
                    .await?
            {
                return Ok(name);
            }
        }
        unreachable!("the loop ends by returning")
    }

    /// `spawn_reporting` without its warnings.
    pub async fn spawn(&self, req: SpawnRequest) -> Result<Agent, SpawnError> {
        Ok(self.spawn_reporting(req).await?.agent)
    }

    /// Starts an agent on a new task folder with zero or more repos. Undoes every step on failure, so a failed
    /// spawn leaves no worktree, branch, task folder, settings file, card or session behind. What went
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
        // nothing saying why. Git makes the worktrees; on Windows Claude Code runs its commands in Git Bash.
        self.opts.tools.require(ToolId::Claude).await?;
        self.opts.tools.require(ToolId::Git).await?;
        if fleet_api::Os::current() == fleet_api::Os::Windows {
            self.opts.tools.require(ToolId::GitBash).await?;
        }
        if req.persona.is_empty() {
            return Err(SpawnError::NoPersona);
        }
        if self.store.product(&req.product).await?.is_none() {
            return Err(RepoError::UnknownProduct(req.product.clone()).into());
        }
        if let Some(release) = req.release {
            let theirs = self.store.release(release).await?.map(|r| r.product);
            if theirs.as_deref() != Some(req.product.as_str()) {
                return Err(SpawnError::ReleaseNotInProduct {
                    release,
                    product: req.product.clone(),
                });
            }
        }
        // Asked first, so a broken login costs nothing either.
        let persona = self.personas.ready_env(&req.persona).await?;
        let fleet = fleet_config(&self.cfg.agent_config_dir).map_err(SpawnError::AgentConfig)?;
        let orchestrator = self.orchestrator_def(&req)?;
        // Every source is resolved before anything is made, so a typo in the third repo costs nothing.
        // Two checkouts of one repo are one workspace: the first wins, as repeated names always did.
        let mut rows: Vec<Resolved> = Vec::with_capacity(req.repos.len());
        let mut warnings = Vec::new();
        for pick in &req.repos {
            let resolved = self.repos.resolve(&req.product, pick, &persona).await?;
            if rows.iter().any(|r| r.row.name == resolved.row.name) {
                continue;
            }
            // Read-only: the checkout is never fetched; the remote is only asked where its branch is.
            if let RepoStart::Checkout(checkout) = &resolved.start {
                if let Some(w) = self
                    .repos
                    .behind_origin(&resolved.row, checkout, &persona)
                    .await
                {
                    warn!(repo = %resolved.row.name, warning = %w, "starting from a checkout behind its URL");
                    warnings.push(w);
                }
            }
            rows.push(resolved);
        }
        let plan = orchestrator
            .map(|def| self.orchestrator_plan(def, &req, &rows))
            .transpose()?;
        // Solo agents never get a model: they run on Claude's own default.
        let model = match &plan {
            Some(p) => {
                self.resolve_model(&req.product, &p.def.name, p.def.model.as_deref())
                    .await?
            }
            None => None,
        };
        // Fetched, not read from the clone as the dialog does: a file just pushed must not be refused. Still before
        // anything is made, so a refusal leaves nothing behind.
        if plan.is_some() {
            for resolved in &rows {
                let row = &resolved.row;
                let base = workflow::creation_base(resolved).await;
                let checked = {
                    let _base = self.repos.lock_base(&resolved.base).await;
                    workflow::fetch(&resolved.base, &base, &persona.vars).await
                };
                if let Err(error) = checked {
                    return Err(SpawnError::Workflow {
                        repo: row.name.clone(),
                        error,
                    });
                }
            }
        }

        let id = uuid::Uuid::new_v4().simple().to_string();
        tracing::Span::current().record("agent_id", id.as_str());
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let slug = git::slugify(req.slug.as_deref().unwrap_or(prompt));
        let session = format!("{SESSION_PREFIX}{}", &id[..8]);

        // The task folder is a plain folder under the tasks root: Claude's trust is inherited from the
        // trusted root, which a git worktree used directly as cwd would not get. Once it exists no other spawn
        // can pick its name, so the lock is not held through the (possibly fetching) attaches below.
        let (workdir, name, _guard) = {
            let _pick = self.pick_lock.lock().await;
            let name = self.free_task_name(&req.product, &slug).await?;
            let workdir = self.cfg.tasks_root.join(&name);
            std::fs::create_dir_all(&self.cfg.tasks_root)?;
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
            internal: None,
        };
        let internal_dir = workdir.join(INTERNAL_DIR);
        // The agent's cwd: the task folder for a solo agent, its own folder inside it for an orchestrator.
        let cwd = match &plan {
            Some(_) => workdir.join(ORCHESTRATOR_DIR),
            None => workdir.clone(),
        };
        if let Err(e) = std::fs::create_dir_all(&cwd) {
            self.undo(&undo, None).await;
            return Err(e.into());
        }

        let mut workspaces = Vec::with_capacity(rows.len());
        for resolved in &rows {
            let row = &resolved.row;
            let (wt, warning) = match self
                .repos
                .attach(resolved, &cwd.join(&row.name), &branch, &persona)
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
                changed: false,
            });
            undo.made.push((resolved.base.clone(), wt));
        }
        // After the repos, so a refused repo never creates the product's internal repo for nothing.
        if plan.is_some() {
            undo.internal = Some((req.product.clone(), name.clone()));
            let env = self.internal_env(&req.product).await;
            match self
                .internal
                .attach_task(
                    &req.product,
                    &name,
                    &internal_dir,
                    env.as_ref().map(|e| e.vars.as_slice()),
                )
                .await
            {
                Ok(warning) => warnings.extend(warning),
                Err(e) => {
                    self.undo(&undo, None).await;
                    return Err(e.into());
                }
            }
        }

        let repo_rules = task_repo_rules(&workspaces);
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
            &plan
                .as_ref()
                .map(|p| p.settings(&internal_dir))
                .unwrap_or_default(),
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
            Some(p) => self.write_orchestrator_files(
                p,
                &cwd,
                &fleet.instructions,
                &workspaces,
                &internal_dir,
                &name,
            ),
            None => {
                write_claude_md(&workdir, &fleet.instructions, &workspaces).map_err(SpawnError::Io)
            }
        }
        .and_then(|()| Ok(write_prompt(&cwd, prompt)?));
        if let Err(e) = files {
            self.undo(&undo, None).await;
            return Err(e);
        }

        // Stored before the session starts: SessionStart can arrive within milliseconds of the process starting.
        let agent = Agent {
            id: AgentId(id.clone()),
            goal: prompt
                .lines()
                .next()
                .unwrap_or(prompt)
                .chars()
                .take(120)
                .collect(),
            workdir: cwd.to_string_lossy().into_owned(),
            workspaces,
            session: session.clone(),
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
            product: req.product.clone(),
            kind: if plan.is_some() {
                AgentKind::Orchestrator
            } else {
                AgentKind::Solo
            },
            task: None,
            role: plan.as_ref().map(|p| p.def.name.clone()),
            step: None,
            model,
            attach: None,
        };
        // Re-checks the persona in the insert's transaction: a delete since `ready_env` saw no task of it.
        // The product and the release are re-checked there too: either a delete of them sees this task, or this
        // insert sees them gone.
        if let Err(e) = self
            .store
            .insert_new_task_agent(&agent, &token, req.release)
            .await
        {
            self.undo(&undo, None).await;
            return Err(match e {
                StoreError::ProductGone(p) => RepoError::UnknownProduct(p).into(),
                StoreError::ReleaseGone { release, product } => {
                    SpawnError::ReleaseNotInProduct { release, product }
                }
                StoreError::Released(version) => SpawnError::Released(version),
                e => agent_insert_error(e),
            });
        }
        if let Some(p) = &plan {
            if let Err(e) = self.record_orchestrator(&agent.id, p, &workdir).await {
                self.undo(&undo, Some((&agent.id, &e.to_string()))).await;
                return Err(e.into());
            }
        }

        persona.seed_first_run().await;
        let vars = agent_vars(&persona, agent.kind);
        let launched = self
            .launch(
                &agent.id,
                &session,
                &cwd,
                &settings,
                &mcp,
                &token,
                &vars,
                agent.model.as_deref(),
                Start::New,
            )
            .await;
        if let Err(e) = launched {
            self.undo(&undo, Some((&agent.id, &e.to_string()))).await;
            return Err(e);
        }
        // Spans are not written to daemon.log, so the event carries the agent id itself.
        info!(agent_id = %id, task = %name, repos = agent.workspaces.len(), "agent spawned");
        Ok(SpawnResponse { agent, warnings })
    }

    /// Starts `claude` in a new session in `cwd`, with the agent's token and env and without any ambient token.
    /// On an error no session is left. Returns the session's output from its first byte.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn launch(
        &self,
        agent: &AgentId,
        session: &str,
        cwd: &Path,
        settings: &Path,
        mcp: &Path,
        token: &str,
        vars: &[(String, String)],
        model: Option<&str>,
        start: Start,
    ) -> Result<broadcast::Receiver<Output>, SpawnError> {
        // `claude[0]` is the tools' claude: where their check found it.
        let program = self.opts.tools.require(ToolId::Claude).await?;
        let mut argv = vec![program.to_string_lossy().into_owned()];
        argv.extend(self.opts.claude.iter().skip(1).cloned());
        argv.extend([
            "--settings".into(),
            settings.to_string_lossy().into_owned(),
            "--mcp-config".into(),
            mcp.to_string_lossy().into_owned(),
        ]);
        // One argv element, checked when the definition or override is saved.
        if let Some(model) = model {
            argv.extend(["--model".into(), model.to_string()]);
        }
        // A prompt is one argv element after `--`, never read as a flag. A new agent's own prompt is in
        // PROMPT_FILE; the command line only points to it.
        let tail: Vec<String> = match start {
            Start::New => {
                let id = uuid::Uuid::new_v4().to_string();
                // Stored before the process starts, so a restart a moment later can still resume it.
                self.store.set_claude_session(agent, &id).await?;
                vec![
                    "--session-id".into(),
                    id,
                    "--".into(),
                    PROMPT_POINTER.into(),
                ]
            }
            Start::Resume {
                claude_session,
                prompt,
            } => {
                let mut t = vec!["--resume".into(), claude_session];
                if let Some(p) = prompt {
                    t.extend(["--".into(), p.into()]);
                }
                t
            }
        };
        argv.extend(tail);
        // The user's real PATH, so a tool installed after the daemon started is found by the session too.
        let path = self.opts.tools.path().into_string().ok();
        // The session's terminal is an xterm emulator; the daemon's own TERM (often none from a desktop launcher)
        // says nothing about it. ConPTY ignores it.
        let mut env = vec![("TERM", "xterm-256color"), ("FLEET_TOKEN", token)];
        env.extend(path.as_deref().map(|p| ("PATH", p)));
        env.extend(MCP_HOLD_ENV);
        env.extend(vars.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        env.extend(
            self.opts
                .extra_env
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str())),
        );
        let ambient: Vec<&str> = ambient_vars().collect();
        Ok(self.sessions.start(
            session,
            Spec {
                argv: &argv,
                cwd,
                env: &env,
                remove_env: &ambient,
            },
        )?)
    }

    /// Only ever runs before the agent's process exists, so the task folder holds nothing but what spawn made.
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
        // Before the folder goes, so git still finds the worktree it is asked to remove.
        if let Some((product, name)) = &u.internal {
            if let Err(e) = self.internal.delete_task_branch(product, name).await {
                kept.push(format!(
                    "branch {} of the internal repo ({})",
                    Internal::task_branch(name),
                    one_line(&e.to_string())
                ));
            }
        }
        let mut failed = Vec::new();
        for (base, wt) in u.made.iter().rev() {
            let _base = self.repos.lock_base(base).await;
            if git::remove(base, wt).await.is_err() {
                failed.push((base, wt));
            }
        }
        remove_dir(&u.workdir, &mut kept);
        // With its folder gone a worktree that git would not remove is only an entry to prune, so it gets one more
        // try; what is still left then is in the base, not the task folder.
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
            .archive_task(id, move |p| {
                p.state = TaskState::Archived;
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

    /// The environment of the persona a task runs under, checked again now; a record from before personas that
    /// none has adopted has none.
    pub(crate) async fn persona_of(&self, a: &Agent) -> Result<PersonaEnv, ServiceError> {
        if a.persona.is_empty() {
            return Err(ServiceError::NoPersona);
        }
        Ok(self.personas.ready_env(&a.persona).await?)
    }

    /// The environment to reach a product's internal remote under, when it has one and its default persona's GitHub
    /// login works; a login that fails is logged and the notes stay local for now.
    pub(crate) async fn internal_env(&self, product: &str) -> Option<PersonaEnv> {
        let row = match self.store.product(product).await {
            Ok(Some(row)) => row,
            Ok(None) => return None,
            Err(e) => {
                warn!(product, error = %e, "cannot read the product for its internal remote");
                return None;
            }
        };
        row.internal_url.as_ref()?;
        let persona = row.default_persona?;
        match self.personas.gh_env(&persona).await {
            Ok(env) => Some(env),
            Err(e) => {
                warn!(product, error = %e, "internal notes stay local: the default persona cannot reach GitHub");
                None
            }
        }
    }

    /// Pushes what an earlier merge or a crash left on each connected product's `main`.
    async fn push_pending_internal_notes(&self) {
        let products = match self.store.products().await {
            Ok(p) => p,
            Err(e) => {
                error!(error = %e, "cannot list the products to push their internal notes");
                return;
            }
        };
        for product in products.iter().filter(|p| p.internal_url.is_some()) {
            let Some(env) = self.internal_env(&product.id).await else {
                continue;
            };
            match self
                .internal
                .push_pending(&product.id, &env.vars, self.opts.clock.now())
                .await
            {
                Ok(report) => {
                    if let PushOutcome::Failed(why) = &report.outcome {
                        warn!(product = %product.id, error = %why, "internal notes not pushed");
                    }
                    if let Some(branch) = &report.parked {
                        warn!(product = %product.id, branch = %branch, "unpushed internal notes conflicted with the remote; merge the branch by hand");
                    }
                }
                Err(e) => {
                    warn!(product = %product.id, error = %e, "cannot push the internal notes")
                }
            }
        }
    }

    /// Adds a repo (name, path or URL) to a running or finished task as `<workdir>/<name>` on the task's
    /// branch, and tells the agent. A repo the task already has is returned unchanged.
    pub async fn attach_repo(&self, id: &AgentId, input: &str) -> Result<Workspace, ServiceError> {
        let Attached { w, warning, new } = self.attach(id, input).await?;
        if !new {
            return Ok(w);
        }
        info!(agent_id = %id.0, repo = %w.repo, "repo attached");
        self.log_task(id, format!("repo {} added", w.repo)).await;
        let warning = warning.map(|w| one_line(&w));
        let mut line = format!(
            "Fleet Manager: repo {} is now available at {} (branch {}).",
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
    /// can take the line; returns whether it did. The line ends with Enter: on a permission prompt that picks
    /// the highlighted "Yes", and at the startup trust dialog (Queued, before any hook) it picks "No, exit" and ends
    /// the agent. Skipping is safe: `CLAUDE.md` lists the repos and `list_repos` answers.
    pub(crate) async fn type_line(&self, a: &Agent, line: &str) -> bool {
        if !reads_text(a) {
            debug!(agent_id = %a.id.0, status = ?a.status, blocked_reason = ?a.blocked_reason, %line, "not typing into agent, a dialog may be up");
            return false;
        }
        if !self.sessions.alive(&a.session) {
            return false;
        }
        let typed = match self.sessions.write_text_async(&a.session, line).await {
            Ok(()) => self.sessions.write_key_async(&a.session, Key::Enter).await,
            Err(e) => Err(e),
        };
        if let Err(e) = typed {
            warn!(agent_id = %a.id.0, error = %e, "cannot type into the agent's session");
            return false;
        }
        true
    }

    /// The attach behind `attach_repo` and `grant_repo`, which tell the agent in different ways.
    #[tracing::instrument(skip_all, fields(agent_id = %id.0))]
    async fn attach(&self, id: &AgentId, input: &str) -> Result<Attached, ServiceError> {
        // Before the lock: the logins are asked through the CLIs, and a task's persona never changes.
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
        // Within the task's product, from the URL when the repo has one, else from its local path.
        let pick = self.repos.default_pick(&a.product, input).await?;
        let resolved = self.repos.resolve(&a.product, &pick, &persona).await?;
        let row = &resolved.row;
        if let Some(w) = a.workspaces.iter().find(|w| w.repo == row.name) {
            return Ok(Attached {
                w: w.clone(),
                warning: None,
                new: false,
            });
        }
        let workdir = PathBuf::from(&a.workdir);
        // An owned repo is on whatever branch the agent chose, so only an attached one names the task branch.
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
            .attach(&resolved, &workdir.join(&row.name), &branch, &persona)
            .await?;
        let w = Workspace {
            repo: row.name.clone(),
            branch: wt.branch.clone(),
            worktree: wt.path.to_string_lossy().into_owned(),
            base: wt.base.clone(),
            from_branch: wt.from_branch.clone(),
            has_changes: false,
            owned: false,
            changed: false,
        };
        let pushed = w.clone();
        if let Err(e) = self
            .store
            .update_agent(id, move |x| x.workspaces.push(pushed))
            .await
        {
            // Killed meanwhile: the worktree has no card to be removed with.
            let _base = self.repos.lock_base(&resolved.base).await;
            let _ = git::remove(&resolved.base, &wt).await;
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
            &task_repo_rules(&all),
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

    /// The workspace for `input` if the task already has it, matched only against what is already known:
    /// an ask must not clone or remember anything before it is granted.
    async fn existing_workspace(
        &self,
        a: &Agent,
        input: &str,
    ) -> Result<Option<Workspace>, ServiceError> {
        let canon = |p: &str| fleet_fs::canonical(Path::new(p)).ok();
        let wanted = canon(input);
        if let Some(w) = a
            .workspaces
            .iter()
            .find(|w| w.repo == input || (wanted.is_some() && canon(&w.worktree) == wanted))
        {
            return Ok(Some(w.clone()));
        }
        let rows = self.store.repos(&a.product).await?;
        let name = rows
            .iter()
            .find(|r| {
                r.url.as_deref() == Some(input)
                    || r.path
                        .as_ref()
                        .is_some_and(|p| wanted.is_some() && canon(&p.to_string_lossy()) == wanted)
            })
            .map(|r| &r.name);
        Ok(name
            .and_then(|n| a.workspaces.iter().find(|w| &w.repo == n))
            .cloned())
    }

    /// The agent's `request_repo`: answers at once when the task has the repo, otherwise puts the card in
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
        self.log_task(id, format!("repo {} granted", w.repo)).await;
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
                "Fleet Manager: granted {} at {} (branch {})",
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
        self.log_task(id, format!("repo request refused: {reason}"))
            .await;
        if !self.answer_held(id, RepoAnswer::Refused(reason.clone())) {
            self.type_line(
                &before,
                &format!("Fleet Manager: refused {}: {reason}", request.repo),
            )
            .await;
        }
        Ok(())
    }

    /// Adds a line to the task's history. The record is secondary to what happened, so a failure is only logged.
    pub(crate) async fn log_task(&self, id: &AgentId, text: String) {
        if let Err(e) = self.store.task_log(id, self.opts.clock.now(), &text).await {
            warn!(agent_id = %id.0, error = %e, "cannot log to the task record");
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

    /// Registers each repo the agent `git init`ed directly in its task folder as an owned workspace, so it
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
        // Added to the task's product; an agent from before products has none to add them to.
        if found.is_empty() || a.product.is_empty() {
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
            let row = match self.repos.remember_owned(&path, &a.product).await {
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
                changed: false,
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
        self.sessions
            .write_key_async(&a.session, Key::CtrlU)
            .await?;
        self.sessions.write_text_async(&a.session, text).await?;
        self.sessions
            .write_key_async(&a.session, Key::Enter)
            .await?;
        Ok(())
    }

    pub async fn pause(&self, id: &AgentId) -> Result<(), ServiceError> {
        let a = self.running(id).await?;
        if a.paused {
            return Ok(());
        }
        self.sessions
            .write_key_async(&a.session, Key::Escape)
            .await?;
        self.store.update_agent(id, |a| a.paused = true).await?;
        Ok(())
    }

    pub async fn resume(&self, id: &AgentId) -> Result<(), ServiceError> {
        let a = self.running(id).await?;
        if !a.paused {
            return Ok(());
        }
        self.sessions
            .write_key_async(&a.session, Key::CtrlU)
            .await?;
        self.sessions
            .write_text_async(&a.session, "continue")
            .await?;
        self.sessions
            .write_key_async(&a.session, Key::Enter)
            .await?;
        self.store.update_agent(id, |a| a.paused = false).await?;
        Ok(())
    }

    /// Ends the session and keeps everything on disk: the worktrees, the branches and the task record. The pane
    /// is saved first, since a stopped agent has no terminal to show. Stopping a stopped agent changes nothing.
    pub async fn stop(&self, id: &AgentId) -> Result<(), ServiceError> {
        let a = self.find(id).await?;
        if a.status == Status::Stopped {
            // Stopped on record over a session that still runs, which reconcile leaves alone: a second Stop is the
            // way out. Its bookkeeping was done by the first.
            if self.sessions.alive(&a.session) {
                self.kill_or_gone(&a).await?;
                self.log_task(task_id(&a), "session ended on retry".into())
                    .await;
                info!(agent_id = %id.0, "stopped agent's session ended on retry");
            }
            return Ok(());
        }
        let live = self.sessions.alive(&a.session);
        // Best effort: a pane that cannot be read must not keep the agent running.
        let screen = if live {
            self.sessions.screen_text(&a.session).ok()
        } else {
            None
        };
        let now = self.opts.clock.now();
        // Written before the kill, so the death it causes (a SessionEnd hook, the reconcile timer) finds the agent
        // already Stopped and cannot name another cause. One write: the task record follows the agent only when
        // the status changes in the same call.
        self.store
            .update_agent(id, |x| {
                x.status = Status::Stopped;
                x.stop_cause = Some(StopCause::Stopped);
                x.activity = "stopped by you".into();
                x.status_since = now;
                x.paused = false;
                // The held call ends below; a request left set would claim one is still waiting.
                x.repo_request = None;
                x.blocked_reason = None;
            })
            .await?;
        let failed = if live {
            self.kill_or_gone(&a).await.err()
        } else {
            None
        };
        let task = task_id(&a);
        // The record's screen is its lead's: a specialist's would replace the one its card shows.
        if let Some(screen) = screen.filter(|_| a.kind != AgentKind::Specialist) {
            // The agent is stopped either way; a lost screen only leaves its terminal saying "not saved".
            if let Err(e) = self
                .store
                .update_task(task, |p| p.last_screen = Some(screen))
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
        self.log_task(task, line).await;
        info!(agent_id = %id.0, "agent stopped");
        // Ends a held request_repo or propose_breakdown call now rather than when its hold runs out.
        self.waiters.lock().expect("waiters poisoned").remove(&id.0);
        self.breakdown_waiters
            .lock()
            .expect("waiters poisoned")
            .remove(&id.0);
        self.request_measure(task);
        // Reported only after the bookkeeping: the agent is Stopped either way, and Archive refuses while the
        // session lives (`SessionStillLive`).
        match failed {
            Some(e) => Err(e.into()),
            None => Ok(()),
        }
    }

    /// Kills the agent's session. A session that is already gone counts as done: it exited on its own between the
    /// stop's check and the kill.
    pub(crate) async fn kill_or_gone(&self, a: &Agent) -> Result<(), SessionError> {
        match self.sessions.kill(&a.session) {
            Ok(()) | Err(SessionError::Missing(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }

    async fn running(&self, id: &AgentId) -> Result<Agent, ServiceError> {
        let a = self.find(id).await?;
        if a.status == Status::Stopped || !self.sessions.alive(&a.session) {
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

    /// Relaunches every agent the last daemon stop or crash interrupted, with `claude --resume` (one never prompted
    /// starts afresh). Each one that
    /// cannot be resumed is stopped with the reason; one failing never stops the others.
    /// Stops as soon as `begin_shutdown` is called: what it has not relaunched is left for the next start.
    pub async fn resume_interrupted(&self) -> Result<(), ServiceError> {
        // Each check runs the persona's CLIs, so it is made once per persona, not once per agent.
        let mut personas = HashMap::new();
        for a in self.store.snapshot().await?.agents {
            if self.shutting_down.load(Ordering::SeqCst) {
                break;
            }
            if a.status == Status::Stopped || self.sessions.alive(&a.session) {
                continue;
            }
            if let Err(why) = self.resume_one(&a, &mut personas).await {
                if let Err(e) = self.unresumable().stop(&a.id, &why).await {
                    error!(agent_id = %a.id.0, error = %e, "cannot stop the agent that could not resume");
                }
            }
        }
        Ok(())
    }

    /// What a daemon start runs once it serves: the end of the repo requests the restart interrupted, then the
    /// resume, then the first reconcile, which must find the resumed sessions alive. The requests go first: once an
    /// agent is resumed, a request it makes is live and must not be cleared.
    pub async fn recover(&self) -> Result<(), ServiceError> {
        // A failed step must not keep the next from running: without the resume every interrupted agent is Crashed.
        if let Err(e) = self.clear_interrupted_requests().await {
            error!(error = %e, "ending the interrupted repo requests failed");
        }
        if let Err(e) = self.resume_interrupted().await {
            error!(error = %e, "resuming the interrupted agents failed");
        }
        self.push_pending_internal_notes().await;
        self.reconcile().await
    }

    /// `Err` says why the agent cannot be resumed; it is empty when there is no Claude session to resume.
    async fn resume_one(
        &self,
        a: &Agent,
        personas: &mut HashMap<String, Result<PersonaEnv, String>>,
    ) -> Result<(), String> {
        // Not prompted yet (still starting, or at the trust dialog): Claude has written no transcript to resume, so
        // it starts over, as at spawn.
        let fresh = a.status == Status::Queued
            || (a.status == Status::NeedsYou && a.blocked_reason == Some(BlockedReason::Startup));
        let claude_session = if fresh {
            None
        } else {
            match self
                .store
                .claude_session(&a.id)
                .await
                .map_err(|e| e.to_string())?
            {
                Some(id) => Some(id),
                None => return Err(String::new()),
            }
        };
        let cwd = PathBuf::from(&a.workdir);
        if !cwd.is_dir() {
            return Err(format!("{} is gone", cwd.display()));
        }
        if fresh && !cwd.join(PROMPT_FILE).is_file() {
            return Err(format!("{} is gone", cwd.join(PROMPT_FILE).display()));
        }
        let dir = self.agent_dir(&a.id.0);
        for f in [dir.join("settings.json"), dir.join("mcp.json")] {
            if !f.is_file() {
                return Err(format!("{} is gone", f.display()));
            }
        }
        let token = self
            .store
            .agent_token(&a.id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("its token is gone")?;
        let (settings, mcp) = hooks_cfg::rebase(&dir, &self.opts.base_url, &a.id, &token)
            .map_err(|e| e.to_string())?;
        let persona = match personas.get(&a.persona) {
            Some(p) => p.clone(),
            None => {
                let p = self.persona_of(a).await.map_err(|e| e.to_string());
                personas.insert(a.persona.clone(), p.clone());
                p
            }
        }?;
        let vars = agent_vars(&persona, a.kind);
        let interrupted_work = !a.paused && matches!(a.status, Status::Working | Status::NeedsYou);
        let start = match claude_session {
            Some(claude_session) => Start::Resume {
                claude_session,
                prompt: interrupted_work.then_some(RESUME_PROMPT),
            },
            None => Start::New,
        };
        if self.shutting_down.load(Ordering::SeqCst) {
            return Ok(());
        }
        // Before the launch, so reconcile never sees the session end before the watcher does.
        self.resuming
            .lock()
            .expect("resuming poisoned")
            .insert(a.id.0.clone());
        let launched = self
            .launch(
                &a.id,
                &a.session,
                &cwd,
                &settings,
                &mcp,
                &token,
                &vars,
                a.model.as_deref(),
                start,
            )
            .await;
        let rx = match launched {
            Ok(rx) => rx,
            Err(e) => {
                self.resuming
                    .lock()
                    .expect("resuming poisoned")
                    .remove(&a.id.0);
                return Err(e.to_string());
            }
        };
        let now = self.opts.clock.now();
        let updated = self
            .store
            .update_agent(&a.id, |x| {
                // Stopped by its owner since the snapshot: the watcher ends it, and this must not revive it.
                if x.status == Status::Stopped {
                    return;
                }
                if fresh {
                    // A new process: nothing of the old one, its pause included, carries over.
                    x.status = Status::Queued;
                    x.status_since = now;
                    x.activity = "starting".into();
                    x.blocked_reason = None;
                    x.paused = false;
                    return;
                }
                x.activity = RESUMED.into();
                // A breakdown awaiting its decision is still on record, and a repo request is
                // `clear_interrupted_requests`' to end (in `recover`, before this); only a dialog on claude's screen
                // is gone with the restart.
                let decision_pending = matches!(
                    x.blocked_reason,
                    Some(BlockedReason::Breakdown | BlockedReason::RepoAccess)
                );
                if interrupted_work && !decision_pending {
                    if x.status != Status::Working {
                        x.status_since = now;
                    }
                    x.status = Status::Working;
                    x.blocked_reason = None;
                    x.repo_request = None;
                }
            })
            .await;
        // After the update, so an exit at once is recorded over "resumed", never under it.
        tokio::spawn(self.unresumable().watch(a.id.clone(), rx));
        if let Err(e) = updated {
            error!(agent_id = %a.id.0, error = %e, "cannot mark the agent resumed");
        }
        info!(agent_id = %a.id.0, "agent resumed after daemon restart");
        Ok(())
    }

    /// Brings the store in line with reality. Safe to call repeatedly: at startup (after resuming the agents a
    /// restart interrupted) and on a timer.
    #[tracing::instrument(skip_all)]
    pub async fn reconcile(&self) -> Result<(), ServiceError> {
        let agents = self.store.snapshot().await?.agents;
        self.reconcile_with(agents).await
    }

    /// Reconcile against an agent list read earlier. Every write re-checks the status it acts on, because a
    /// hook can land between the read and the write, and one agent failing never stops the others.
    pub async fn reconcile_with(&self, agents: Vec<Agent>) -> Result<(), ServiceError> {
        let live: HashSet<String> = self.sessions.list().into_iter().collect();
        // Read after the list: a shutdown that began before the sessions were killed is seen here.
        if self.shutting_down.load(Ordering::SeqCst) {
            return Ok(());
        }
        let now = self.opts.clock.now();
        for a in agents {
            if a.status == Status::Stopped
                || self
                    .spawning
                    .lock()
                    .expect("spawning set poisoned")
                    .contains_key(&a.id.0)
                || self
                    .resuming
                    .lock()
                    .expect("resuming poisoned")
                    .contains(&a.id.0)
            {
                continue;
            }
            if let Err(e) = self.reconcile_one(&a, live.contains(&a.session), now).await {
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

/// Whether nothing but the input box of `a` (as read by the caller) can take a typed line.
pub(crate) fn reads_text(a: &Agent) -> bool {
    match a.status {
        Status::Working | Status::Review => true,
        // Both wait on a held call that has returned or will: the agent is at its input box.
        Status::NeedsYou => matches!(
            a.blocked_reason,
            Some(BlockedReason::RepoAccess | BlockedReason::Breakdown)
        ),
        // Talking is not produced yet; until it is, nothing is known about what its screen shows.
        Status::Queued | Status::Talking | Status::Stopped => false,
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
