//! The agent lifecycle: spawn, answer, pause, resume, kill, and reconcile with what tmux really runs.
use crate::config::Config;
use crate::git::{self, GitError, Worktree};
use crate::hooks_cfg::{write_mcp_config, write_settings, SettingsError};
use crate::repos::{RepoError, Repos, Resolved};
use crate::store::{RepoRow, Store, StoreError};
use crate::tmux::{Tmux, TmuxError};
use fleet_api::{
    Agent, AgentId, BlockedReason, DiffReport, KillReport, RepoRequest, SpawnRequest, Status, Workspace, WorkspaceDiff,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;
use tracing::{debug, error, info, warn};

pub const SESSION_PREFIX: &str = "fleet-";
const MAX_PROMPT_BYTES: usize = 20_000;
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
}

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("no such agent")]
    NotFound,
    #[error("agent is not running")]
    NotRunning,
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
    #[error("no repo request is waiting for an answer")]
    NoPendingRequest,
    #[error("a repo request is already waiting for an answer")]
    RequestPending,
    #[error("the repo request was withdrawn")]
    RequestWithdrawn,
    #[error("no answer yet; the decision will be typed into this session when it is made")]
    NoAnswerYet,
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
pub fn unapplied_repo_settings(worktree: &std::path::Path) -> Vec<String> {
    [".claude/settings.json", ".claude/settings.local.json", ".mcp.json"]
        .into_iter()
        .filter(|f| worktree.join(f).is_file())
        .map(String::from)
        .collect()
}

const START_A_REPO: &str = "To start a new repo, create a subfolder and run `git init` in it.";
const ASK_FOR_A_REPO: &str =
    "To work in an existing repo, call the `request_repo` tool (fleet MCP server); never clone or copy repos yourself.";

/// The subfolder a workspace lives in, relative to the project folder.
fn folder(w: &Workspace) -> String {
    Path::new(&w.worktree).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| w.repo.clone())
}

/// `CLAUDE.md` for a project folder: where the repos are, how to get more, plus an import of each repo's own `CLAUDE.md`.
fn project_claude_md(workspaces: &[Workspace]) -> String {
    let mut text = String::from("# fleet-manager project\n\nThis folder is a project folder, not a git repository.");
    if workspaces.is_empty() {
        text.push_str(" This project has no repos yet.\n");
    } else {
        text.push_str(" Each repo of this project is a subfolder:\n\n");
        for w in workspaces {
            text.push_str(&format!("- `{}/` (branch `{}`)\n", folder(w), w.branch));
        }
        text.push_str("\nMake every change inside those subfolders and run git there. Do not create files directly in this folder.\n");
    }
    text.push_str(&format!("\n{START_A_REPO}\n{ASK_FOR_A_REPO}\n"));
    for w in workspaces {
        if Path::new(&w.worktree).join("CLAUDE.md").is_file() {
            text.push_str(&format!("\n@{}/CLAUDE.md\n", folder(w)));
        }
    }
    text
}

fn write_claude_md(workdir: &Path, workspaces: &[Workspace]) -> std::io::Result<()> {
    std::fs::write(workdir.join("CLAUDE.md"), project_claude_md(workspaces))
}

/// What spawn must remove again if a later step fails.
struct Undo {
    /// Each worktree made so far, with the base it hangs off.
    made: Vec<(PathBuf, Worktree)>,
    workdir: PathBuf,
    settings_dir: PathBuf,
}

/// Deletes `dir` only when it is empty, so nothing the daemon did not make is ever removed; otherwise says what is left.
fn remove_if_empty(dir: &Path) -> Option<String> {
    let err = match std::fs::remove_dir(dir) {
        Ok(()) => return None,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => e,
    };
    let mut names: Vec<String> = match std::fs::read_dir(dir) {
        Ok(entries) => entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect(),
        Err(_) => vec![],
    };
    if names.is_empty() {
        return Some(format!("kept {}: {err}", dir.display()));
    }
    names.sort();
    Some(format!("kept {}: it contains {}", dir.display(), names.join(", ")))
}

/// tmux types a newline as Enter, which would submit half a line.
/// Control characters (escape sequences, Ctrl keys) would act on the agent's terminal rather than be read by it.
fn one_line(text: &str) -> String {
    let clean: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    clean.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub struct AgentService {
    cfg: Config,
    store: Arc<Store>,
    repos: Arc<Repos>,
    tmux: Tmux,
    opts: ServiceOptions,
    /// Agents between "stored" and "tmux session exists"; reconcile must not call them lost.
    spawning: Mutex<HashSet<String>>,
    /// Held from choosing a project directory until it exists, so two spawns never pick the same name.
    pick_lock: tokio::sync::Mutex<()>,
    /// Held while a project's workspace list is read, extended and written back (attach, adopt), so two
    /// changes never both push from the same stale list.
    workspaces_lock: tokio::sync::Mutex<()>,
    /// Held while a repo request is opened or decided, so a grant never races the request it answers (or another
    /// decision on it). Decisions hold it through the attach, which may fetch.
    requests_lock: tokio::sync::Mutex<()>,
    /// The answer channel of each held `request_repo` call, by agent id. An entry may outlive its call (the client
    /// gave up, the hold ran out): sending on it then fails, which is how a decision knows to type instead.
    waiters: Mutex<HashMap<String, oneshot::Sender<RepoAnswer>>>,
}

struct SpawningGuard<'a>(&'a Mutex<HashSet<String>>, String);

impl Drop for SpawningGuard<'_> {
    fn drop(&mut self) {
        self.0.lock().expect("spawning set poisoned").remove(&self.1);
    }
}

impl AgentService {
    pub fn new(cfg: Config, store: Arc<Store>, repos: Arc<Repos>, tmux: Tmux, opts: ServiceOptions) -> AgentService {
        AgentService {
            cfg,
            store,
            repos,
            tmux,
            opts,
            spawning: Mutex::new(HashSet::new()),
            pick_lock: tokio::sync::Mutex::new(()),
            workspaces_lock: tokio::sync::Mutex::new(()),
            requests_lock: tokio::sync::Mutex::new(()),
            waiters: Mutex::new(HashMap::new()),
        }
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    pub fn options(&self) -> &ServiceOptions {
        &self.opts
    }

    fn agent_dir(&self, id: &str) -> PathBuf {
        self.cfg.state_dir.join("agents").join(id)
    }

    pub async fn agent(&self, id: &AgentId) -> Result<Agent, ServiceError> {
        self.find(id).await
    }

    pub async fn remembered_repos(&self) -> Result<Vec<RepoRow>, ServiceError> {
        Ok(self.store.repos().await?)
    }

    async fn find(&self, id: &AgentId) -> Result<Agent, ServiceError> {
        let snap = self.store.snapshot().await?;
        snap.agents.into_iter().find(|a| &a.id == id).ok_or(ServiceError::NotFound)
    }

    /// First free `<slug>`, `<slug>-2`, ...: no such project folder and no `<prefix>/<name>` branch in any remembered
    /// repo, since Kill keeps branches and any of those repos may be attached to this project later.
    async fn free_project_name(&self, slug: &str) -> Result<String, SpawnError> {
        let repos = self.store.repos().await?;
        for n in 1.. {
            let name = if n == 1 { slug.to_string() } else { format!("{slug}-{n}") };
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

    /// Starts an agent on a new project folder with zero or more repos. Undoes every step on failure, so a failed
    /// spawn leaves no worktree, branch, project folder, settings file, card or tmux session behind.
    #[tracing::instrument(skip_all, fields(agent_id = tracing::field::Empty))]
    pub async fn spawn(&self, req: SpawnRequest) -> Result<Agent, SpawnError> {
        let prompt = req.prompt.trim();
        if prompt.is_empty() {
            return Err(SpawnError::EmptyPrompt);
        }
        if prompt.len() > MAX_PROMPT_BYTES {
            return Err(SpawnError::PromptTooLong);
        }
        // Every source is resolved before anything is made, so a typo in the third repo costs nothing.
        // Two checkouts of one repo are one workspace: the first wins, as repeated names always did.
        let mut rows: Vec<Resolved> = Vec::with_capacity(req.repos.len());
        for input in &req.repos {
            let resolved = self.repos.resolve(input.trim()).await?;
            if !rows.iter().any(|r| r.row.name == resolved.row.name) {
                rows.push(resolved);
            }
        }

        let id = uuid::Uuid::new_v4().simple().to_string();
        tracing::Span::current().record("agent_id", id.as_str());
        let token = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
        let slug = git::slugify(req.task_slug.as_deref().unwrap_or(prompt));
        let session = format!("{SESSION_PREFIX}{}", &id[..8]);

        // The project folder is a plain folder under the projects root: Claude's trust is inherited from the
        // trusted root, which a git worktree used directly as cwd would not get. Once it exists no other spawn
        // can pick its name, so the lock is not held through the (possibly fetching) attaches below.
        let (workdir, name) = {
            let _pick = self.pick_lock.lock().await;
            let name = self.free_project_name(&slug).await?;
            let workdir = self.cfg.projects_root.join(&name);
            std::fs::create_dir_all(&self.cfg.projects_root)?;
            // Not create_dir_all: a folder that appeared meanwhile is not ours, and undo deletes this one.
            std::fs::create_dir(&workdir)?;
            (workdir, name)
        };
        let branch = format!("{}/{name}", self.cfg.branch_prefix);
        let settings_dir = self.agent_dir(&id);
        let mut undo = Undo { made: vec![], workdir: workdir.clone(), settings_dir: settings_dir.clone() };

        let mut workspaces = Vec::with_capacity(rows.len());
        for Resolved { row, start } in &rows {
            let (wt, warning) = match self.repos.attach(row, start, &workdir.join(&row.name), &branch).await {
                Ok(x) => x,
                Err(e) => {
                    self.undo(&undo, None).await;
                    return Err(e.into());
                }
            };
            if let Some(w) = warning {
                warn!(repo = %row.name, warning = %w, "repo attached with a warning");
            }
            for f in unapplied_repo_settings(&wt.path) {
                warn!(repo = %row.name, setting = %f, "repo setting is not loaded by Claude Code from the project directory; its rules do not apply to this agent");
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

        let written = write_settings(&settings_dir, &self.opts.base_url, &AgentId(id.clone()))
            .and_then(|s| Ok((s, write_mcp_config(&settings_dir, &self.opts.base_url, &AgentId(id.clone()), &token)?)));
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
        if let Err(e) = write_claude_md(&workdir, &workspaces) {
            self.undo(&undo, None).await;
            return Err(SpawnError::Io(e));
        }

        // Stored before tmux starts: SessionStart can arrive within milliseconds of the process starting.
        self.spawning.lock().expect("spawning set poisoned").insert(id.clone());
        let _guard = SpawningGuard(&self.spawning, id.clone());
        let agent = Agent {
            id: AgentId(id.clone()),
            task: prompt.lines().next().unwrap_or(prompt).chars().take(120).collect(),
            workdir: workdir.to_string_lossy().into_owned(),
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
        };
        if let Err(e) = self.store.insert_agent(&agent, &token).await {
            self.undo(&undo, None).await;
            return Err(e.into());
        }

        // The prompt is one argv element after `--`: no shell sees it and it cannot be read as a flag.
        let mut argv = self.opts.claude.clone();
        argv.extend([
            "--settings".into(),
            settings.to_string_lossy().into_owned(),
            "--mcp-config".into(),
            mcp.to_string_lossy().into_owned(),
            "--".into(),
            prompt.to_string(),
        ]);
        let mut env = vec![("FLEET_TOKEN", token.as_str())];
        env.extend(MCP_HOLD_ENV);
        if let Err(e) = self.tmux.new_session(&session, &workdir, &env, &argv).await {
            self.undo(&undo, Some(&agent.id)).await;
            return Err(e.into());
        }
        // Spans are not written to daemon.log, so the event carries the agent id itself.
        info!(agent_id = %id, project = %name, repos = agent.workspaces.len(), "agent spawned");
        Ok(agent)
    }

    /// Only ever runs before the agent's process exists, so the project folder holds nothing but what spawn made.
    async fn undo(&self, u: &Undo, stored: Option<&AgentId>) {
        if let Some(id) = stored {
            let _ = self.store.remove_agent(id).await;
        }
        let _ = std::fs::remove_dir_all(&u.settings_dir);
        for (base, wt) in u.made.iter().rev() {
            let _base = self.repos.lock_base(base).await;
            let _ = git::remove(base, wt).await;
        }
        let _ = std::fs::remove_dir_all(&u.workdir);
    }

    /// Adds a repo (name, path or URL) to a running or finished project as `<workdir>/<name>` on the project's
    /// branch, and tells the agent. A repo the project already has is returned unchanged.
    pub async fn attach_repo(&self, id: &AgentId, input: &str) -> Result<Workspace, ServiceError> {
        let Attached { w, warning, new } = self.attach(id, input).await?;
        if !new {
            return Ok(w);
        }
        info!(agent_id = %id.0, repo = %w.repo, "repo attached");
        let warning = warning.map(|w| one_line(&w));
        let mut line = format!("fleet-manager: repo {} is now available at {} (branch {}).", w.repo, w.worktree, w.branch);
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
    async fn type_line(&self, a: &Agent, line: &str) -> bool {
        let reads_text = match a.status {
            Status::Working | Status::Review => true,
            Status::NeedsYou => a.blocked_reason == Some(BlockedReason::RepoAccess),
            // Talking is not produced yet; until it is, nothing is known about what its screen shows.
            Status::Queued | Status::Talking | Status::Dead => false,
        };
        if !reads_text {
            debug!(agent_id = %a.id.0, status = ?a.status, blocked_reason = ?a.blocked_reason, %line, "not typing into agent, a dialog may be up");
            return false;
        }
        if !self.tmux.has_session(&a.tmux_session).await.unwrap_or(false) {
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
        let _ws = self.workspaces_lock.lock().await;
        let a = self.find(id).await?;
        let input = input.trim();
        if let Some(w) = a.workspaces.iter().find(|w| w.repo == input) {
            return Ok(Attached { w: w.clone(), warning: None, new: false });
        }
        let Resolved { row, start } = self.repos.resolve(input).await?;
        if let Some(w) = a.workspaces.iter().find(|w| w.repo == row.name) {
            return Ok(Attached { w: w.clone(), warning: None, new: false });
        }
        let workdir = PathBuf::from(&a.workdir);
        // An owned repo is on whatever branch the agent chose, so only an attached one names the project branch.
        let branch = match a.workspaces.iter().find(|w| !w.owned) {
            Some(w) => w.branch.clone(),
            None => {
                let name = workdir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                format!("{}/{name}", self.cfg.branch_prefix)
            }
        };
        let (wt, warning) = self.repos.attach(&row, &start, &workdir.join(&row.name), &branch).await?;
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
        if let Err(e) = self.store.update_agent(id, move |x| x.workspaces.push(pushed)).await {
            // Killed meanwhile: the worktree has no card to be removed with.
            let _base = self.repos.lock_base(&row.base).await;
            let _ = git::remove(&row.base, &wt).await;
            return Err(e.into());
        }
        let mut all = a.workspaces.clone();
        all.push(w.clone());
        if let Err(e) = write_claude_md(&workdir, &all) {
            warn!(agent_id = %id.0, path = %workdir.join("CLAUDE.md").display(), error = %e, "cannot rewrite CLAUDE.md");
        }
        Ok(Attached { w, warning, new: true })
    }

    /// The workspace for `input` if the project already has it, matched only against what is already known:
    /// an ask must not clone or remember anything before it is granted.
    async fn existing_workspace(&self, a: &Agent, input: &str) -> Result<Option<Workspace>, ServiceError> {
        let canon = |p: &str| std::fs::canonicalize(p).ok();
        let wanted = canon(input);
        if let Some(w) = a.workspaces.iter().find(|w| w.repo == input || (wanted.is_some() && canon(&w.worktree) == wanted)) {
            return Ok(Some(w.clone()));
        }
        let rows = self.store.repos().await?;
        let name = rows.iter().find(|r| r.location == input || (wanted.is_some() && canon(&r.location) == wanted)).map(|r| &r.name);
        Ok(name.and_then(|n| a.workspaces.iter().find(|w| &w.repo == n)).cloned())
    }

    /// The agent's `request_repo`: answers at once when the project has the repo, otherwise puts the card in
    /// Needs you and holds until the owner decides (or the hold runs out, see `REPO_REQUEST_HOLD`).
    pub async fn request_repo(&self, id: &AgentId, repo: &str, why: &str) -> Result<RepoAnswer, ServiceError> {
        // Both end up on the card and in a line typed into the session.
        let (repo, why) = (one_line(repo), one_line(why));
        if repo.is_empty() {
            return Err(ServiceError::EmptyText);
        }
        let rx = {
            let _req = self.requests_lock.lock().await;
            let a = self.find(id).await?;
            if let Some(w) = self.existing_workspace(&a, &repo).await? {
                return Ok(RepoAnswer::Granted(w));
            }
            let now = self.opts.clock.now();
            let mut pending = false;
            let request = RepoRequest { repo: repo.clone(), why };
            self.store
                .update_agent(id, |x| {
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
            if pending {
                return Err(ServiceError::RequestPending);
            }
            let (tx, rx) = oneshot::channel();
            self.waiters.lock().expect("waiters poisoned").insert(id.0.clone(), tx);
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
        // Read before settle moves the card to Working: the fallback below may only type onto the request itself.
        let before = self.find(id).await?;
        self.settle(id, format!("granted repo {}", w.repo)).await?;
        let warning = warning.map(|w| one_line(&w));
        if self.answer_held(id, RepoAnswer::Granted(w.clone())) {
            if let Some(w) = &warning {
                warn!(agent_id = %id.0, warning = %w, "repo granted with a warning");
            }
        } else {
            let mut line = format!("fleet-manager: granted {} at {} (branch {})", w.repo, w.worktree, w.branch);
            if let Some(warn) = &warning {
                line.push_str(&format!(" Warning: {warn}"));
            }
            self.type_line(&before, &line).await;
        }
        Ok(w)
    }

    pub async fn refuse_repo(&self, id: &AgentId, reason: &str) -> Result<(), ServiceError> {
        let _req = self.requests_lock.lock().await;
        let Some(request) = self.find(id).await?.repo_request else { return Err(ServiceError::NoPendingRequest) };
        let reason = match one_line(reason) {
            r if r.is_empty() => "no reason given".to_string(),
            r => r,
        };
        let before = self.find(id).await?;
        self.settle(id, format!("refused repo {}", request.repo)).await?;
        // What the agent asked for may be a URL with credentials in it.
        info!(agent_id = %id.0, repo = %crate::repos::log_name(&request.repo), "repo request refused");
        if !self.answer_held(id, RepoAnswer::Refused(reason.clone())) {
            self.type_line(&before, &format!("fleet-manager: refused {}: {reason}", request.repo)).await;
        }
        Ok(())
    }

    /// Ends a decided request on the card; the hooks move the status on from here.
    async fn settle(&self, id: &AgentId, activity: String) -> Result<(), ServiceError> {
        let now = self.opts.clock.now();
        self.store
            .update_agent(id, |x| {
                x.repo_request = None;
                if x.status == Status::Dead {
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
        self.waiters.lock().expect("waiters poisoned").get(&id.0).is_some_and(|tx| !tx.is_closed())
    }

    /// Hands the answer to the held call; false when there is none any more (gone, timed out, never made).
    fn answer_held(&self, id: &AgentId, answer: RepoAnswer) -> bool {
        let tx = self.waiters.lock().expect("waiters poisoned").remove(&id.0);
        tx.is_some_and(|tx| tx.send(answer).is_ok())
    }

    /// A held call does not survive a daemon restart, so a request stored before it can never be answered.
    pub async fn clear_interrupted_requests(&self) -> Result<(), ServiceError> {
        let now = self.opts.clock.now();
        for a in self.store.snapshot().await?.agents.into_iter().filter(|a| a.repo_request.is_some()) {
            let r = self
                .store
                .update_agent(&a.id, |x| {
                    if x.repo_request.take().is_none() || x.status == Status::Dead {
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
    /// shows on the card and in Diff, and so Kill knows never to remove it.
    pub async fn adopt_owned_repos(&self, a: &Agent) -> Result<(), ServiceError> {
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
        if found.is_empty() {
            return Ok(());
        }
        found.sort();
        // An attach in progress holds this through a fetch; the next reconcile picks the repo up instead.
        let Ok(_ws) = self.workspaces_lock.try_lock() else { return Ok(()) };
        let current = self.find(&a.id).await?;
        let mut added = Vec::new();
        for path in found.into_iter().filter(|p| !known(&current.workspaces, p)) {
            let row = match self.repos.remember_owned(&path).await {
                Ok(r) => r,
                Err(e) => {
                    warn!(agent_id = %a.id.0, path = %path.display(), error = %e, "cannot adopt repo");
                    continue;
                }
            };
            let branch = git::current_branch(&path).await.ok().flatten().unwrap_or_else(|| "HEAD".into());
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
        self.store.update_agent(&a.id, move |x| x.workspaces.extend(pushed)).await?;
        let mut all = current.workspaces;
        all.extend(added);
        write_claude_md(&workdir, &all)?;
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

    /// Removes the worktrees (from their bases; their branches stay) and the daemon's files, then the project folder
    /// only if that leaves it empty. An owned repo, or anything else the daemon did not make, keeps the folder and is reported.
    pub async fn kill(&self, id: &AgentId) -> Result<KillReport, ServiceError> {
        let _ws = self.workspaces_lock.lock().await;
        let a = self.find(id).await?;
        if self.tmux.has_session(&a.tmux_session).await? {
            self.tmux.kill(&a.tmux_session).await?;
        }
        for w in a.workspaces.iter().filter(|w| !w.owned) {
            let Some(row) = self.store.repo(&w.repo).await? else {
                warn!(agent_id = %id.0, repo = %w.repo, worktree = %w.worktree, "repo is not remembered on kill; leaving its worktree in place");
                continue;
            };
            let _base = self.repos.lock_base(&row.base).await;
            let wt = Worktree { path: PathBuf::from(&w.worktree), branch: w.branch.clone(), base: w.base.clone(), from_branch: w.from_branch.clone() };
            git::remove_worktree(&row.base, &wt).await?;
        }
        let workdir = PathBuf::from(&a.workdir);
        let _ = std::fs::remove_file(workdir.join("CLAUDE.md"));
        let _ = std::fs::remove_dir_all(self.agent_dir(&id.0));
        let kept = remove_if_empty(&workdir);
        self.store.remove_agent(id).await?;
        info!(agent_id = %id.0, folder = if kept.is_some() { "kept" } else { "removed" }, "agent killed");
        // Ends a held request_repo call now rather than when its hold runs out.
        self.waiters.lock().expect("waiters poisoned").remove(&id.0);
        Ok(KillReport { kept })
    }

    async fn running(&self, id: &AgentId) -> Result<Agent, ServiceError> {
        let a = self.find(id).await?;
        if a.status == Status::Dead || !self.tmux.has_session(&a.tmux_session).await? {
            return Err(ServiceError::NotRunning);
        }
        Ok(a)
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
        let live: HashSet<String> = self.tmux.list_sessions(SESSION_PREFIX).await?.into_iter().collect();
        let now = self.opts.clock.now();
        for a in agents {
            if a.status == Status::Dead || self.spawning.lock().expect("spawning set poisoned").contains(&a.id.0) {
                continue;
            }
            if let Err(e) = self.reconcile_one(&a, live.contains(&a.tmux_session), now).await {
                error!(agent_id = %a.id.0, error = %e, "reconcile failed");
            }
        }
        Ok(())
    }

    async fn reconcile_one(&self, a: &Agent, session_alive: bool, now: i64) -> Result<(), ServiceError> {
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
                    if x.status != Status::Dead {
                        died = true;
                        x.status = Status::Dead;
                        x.activity = "session lost".into();
                        x.blocked_reason = None;
                        x.repo_request = None;
                        x.status_since = now;
                    }
                })
                .await?;
            if died {
                info!(agent_id = %a.id.0, "agent session lost");
            }
            self.waiters.lock().expect("waiters poisoned").remove(&a.id.0);
        } else if a.status == Status::Queued && now - a.status_since >= self.opts.startup_grace_secs {
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
                flags.push(git::has_changes(std::path::Path::new(&w.worktree)).await.unwrap_or(w.has_changes));
            }
            if flags.iter().zip(&a.workspaces).any(|(f, w)| *f != w.has_changes) {
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
        let live: HashSet<String> = self.tmux.list_sessions(SESSION_PREFIX).await?.into_iter().collect();
        for a in self.store.snapshot().await?.agents {
            if live.contains(&a.tmux_session) && matches!(a.status, Status::Queued | Status::Working | Status::NeedsYou) {
                let r = self
                    .store
                    .update_agent(&a.id, |x| {
                        if matches!(x.status, Status::Queued | Status::Working | Status::NeedsYou) {
                            x.activity = "status unverified since daemon restart".into();
                        }
                    })
                    .await;
                if let Err(e) = r {
                    error!(agent_id = %a.id.0, error = %e, "cannot mark the agent unverified");
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
            workspaces.push(WorkspaceDiff { repo: w.repo.clone(), diff: d.text, truncated: d.truncated });
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
