//! The agent lifecycle: spawn, answer, pause, resume, kill, and reconcile with what tmux really runs.
use crate::config::{Config, Repo};
use crate::git::{self, GitError, Worktree};
use crate::hooks_cfg::{write_settings, SettingsError};
use crate::store::{Store, StoreError};
use crate::tmux::{Tmux, TmuxError};
use fleet_api::{Agent, AgentId, BlockedReason, DiffReport, SpawnRequest, Status, Workspace, WorkspaceDiff};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub const SESSION_PREFIX: &str = "fleet-";
const MAX_PROMPT_BYTES: usize = 20_000;
const MAX_DIFF_BYTES: usize = 1 << 20;

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
}

impl ServiceOptions {
    pub fn new(base_url: &str) -> ServiceOptions {
        ServiceOptions {
            claude: vec!["claude".into()],
            base_url: base_url.into(),
            startup_grace_secs: 20,
            clock: Arc::new(SystemClock),
            owner: std::env::var("USER").unwrap_or_else(|_| "local".into()),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    #[error("unknown repo {0:?}")]
    UnknownRepo(String),
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
    #[error("repo {0:?} is no longer in the config")]
    UnknownRepo(String),
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Tmux(#[from] TmuxError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Claude Code settings inside a checkout that will not apply, because Claude starts one level above it.
pub fn unapplied_repo_settings(worktree: &std::path::Path) -> Vec<String> {
    [".claude/settings.json", ".claude/settings.local.json", ".mcp.json"]
        .into_iter()
        .filter(|f| worktree.join(f).is_file())
        .map(String::from)
        .collect()
}

/// `CLAUDE.md` for a project directory: where the repos are, plus an import of each repo's own `CLAUDE.md`.
fn project_claude_md(repo: &str, wt: &Worktree) -> String {
    let mut text = format!(
        "# fleet-board project\n\nThis folder is a project folder, not a git repository. Each repo of this project is checked out as a subdirectory:\n\n- `{repo}/` (branch `{}`)\n\nMake every change inside those subdirectories and run git there. Do not create files directly in this folder.\n",
        wt.branch
    );
    if wt.path.join("CLAUDE.md").is_file() {
        text.push_str(&format!("\n@{repo}/CLAUDE.md\n"));
    }
    text
}

/// What spawn must remove again if a later step fails.
struct Undo<'a> {
    repo: &'a std::path::Path,
    wt: &'a Worktree,
    workdir: &'a std::path::Path,
    settings_dir: &'a std::path::Path,
}

pub struct AgentService {
    cfg: Config,
    store: Arc<Store>,
    tmux: Tmux,
    opts: ServiceOptions,
    /// Agents between "stored" and "tmux session exists"; reconcile must not call them lost.
    spawning: Mutex<HashSet<String>>,
    /// Held from choosing a project directory until it exists, so two spawns never pick the same name.
    pick_lock: tokio::sync::Mutex<()>,
}

struct SpawningGuard<'a>(&'a Mutex<HashSet<String>>, String);

impl Drop for SpawningGuard<'_> {
    fn drop(&mut self) {
        self.0.lock().expect("spawning set poisoned").remove(&self.1);
    }
}

impl AgentService {
    pub fn new(cfg: Config, store: Arc<Store>, tmux: Tmux, opts: ServiceOptions) -> AgentService {
        AgentService { cfg, store, tmux, opts, spawning: Mutex::new(HashSet::new()), pick_lock: tokio::sync::Mutex::new(()) }
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

    async fn find(&self, id: &AgentId) -> Result<Agent, ServiceError> {
        let snap = self.store.snapshot().await?;
        snap.agents.into_iter().find(|a| &a.id == id).ok_or(ServiceError::NotFound)
    }

    fn repo(&self, name: &str) -> Option<&Repo> {
        self.cfg.repos.iter().find(|r| r.name == name)
    }

    /// First free `<slug>`, `<slug>-2`, ...: no such project directory and no such branch in the repo.
    async fn free_project_name(&self, repo: &Repo, slug: &str) -> Result<String, GitError> {
        let root = self.cfg.state_dir.join("worktrees");
        for n in 1.. {
            let name = if n == 1 { slug.to_string() } else { format!("{slug}-{n}") };
            let taken_branch = git::branch_exists(&repo.path, &format!("{}/{name}", self.cfg.branch_prefix)).await?;
            if !root.join(&name).exists() && !taken_branch {
                return Ok(name);
            }
        }
        unreachable!("the loop ends by returning")
    }

    /// Starts an agent on a new project in `req.repo`. Undoes every step on failure, so a failed spawn
    /// leaves no worktree, branch, project directory, settings file, card or tmux session behind.
    pub async fn spawn(&self, req: SpawnRequest) -> Result<Agent, SpawnError> {
        let repo = self.repo(&req.repo).ok_or_else(|| SpawnError::UnknownRepo(req.repo.clone()))?;
        let prompt = req.prompt.trim();
        if prompt.is_empty() {
            return Err(SpawnError::EmptyPrompt);
        }
        if prompt.len() > MAX_PROMPT_BYTES {
            return Err(SpawnError::PromptTooLong);
        }

        let id = uuid::Uuid::new_v4().simple().to_string();
        let token = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
        let slug = git::slugify(req.task_slug.as_deref().unwrap_or(prompt));
        let session = format!("{SESSION_PREFIX}{}", &id[..8]);

        // The project directory is a plain directory above the checkout: Claude's trust is inherited by it
        // from the trusted worktrees root, which a git worktree used directly as cwd would not get.
        let (workdir, wt) = {
            let _pick = self.pick_lock.lock().await;
            let name = self.free_project_name(repo, &slug).await?;
            let workdir = self.cfg.state_dir.join("worktrees").join(&name);
            let wt = git::create(&repo.path, &self.cfg.branch_prefix, &name, &workdir.join(&repo.name)).await?;
            (workdir, wt)
        };
        let settings_dir = self.agent_dir(&id);
        let undo = Undo { repo: &repo.path, wt: &wt, workdir: &workdir, settings_dir: &settings_dir };

        let settings = match write_settings(&settings_dir, &self.opts.base_url, &AgentId(id.clone())) {
            Ok(p) => p,
            Err(e) => {
                self.undo(&undo, None).await;
                return Err(e.into());
            }
        };
        // The agent starts one level above the checkout, so without this it would write files next to the repo
        // instead of in it (seen on the real CLI). Claude also loads no CLAUDE.md from one level down (measured),
        // so each repo's own is imported. The file belongs to the daemon, not to any repo.
        if let Err(e) = std::fs::write(workdir.join("CLAUDE.md"), project_claude_md(&repo.name, &wt)) {
            self.undo(&undo, None).await;
            return Err(SpawnError::Io(e));
        }

        for f in unapplied_repo_settings(&wt.path) {
            eprintln!(
                "fleet-daemon: warning: repo {} has {f}, which Claude Code does not load from the project directory; its rules do not apply to this agent",
                repo.name
            );
        }

        // Stored before tmux starts: SessionStart can arrive within milliseconds of the process starting.
        self.spawning.lock().expect("spawning set poisoned").insert(id.clone());
        let _guard = SpawningGuard(&self.spawning, id.clone());
        let agent = Agent {
            id: AgentId(id.clone()),
            task: prompt.lines().next().unwrap_or(prompt).chars().take(120).collect(),
            workdir: workdir.to_string_lossy().into_owned(),
            workspaces: vec![Workspace {
                repo: repo.name.clone(),
                branch: wt.branch.clone(),
                worktree: wt.path.to_string_lossy().into_owned(),
                base: wt.base.clone(),
                has_changes: false,
            }],
            tmux_session: session.clone(),
            status: Status::Queued,
            activity: "starting".into(),
            status_since: self.opts.clock.now(),
            has_changes: false,
            paused: false,
            owner: self.opts.owner.clone(),
            blocked_reason: None,
        };
        if let Err(e) = self.store.insert_agent(&agent, &token).await {
            self.undo(&undo, None).await;
            return Err(e.into());
        }

        // The prompt is one argv element after `--`: no shell sees it and it cannot be read as a flag.
        let mut argv = self.opts.claude.clone();
        argv.extend(["--settings".into(), settings.to_string_lossy().into_owned(), "--".into(), prompt.to_string()]);
        if let Err(e) = self.tmux.new_session(&session, &workdir, &[("FLEET_TOKEN", &token)], &argv).await {
            self.undo(&undo, Some(&agent.id)).await;
            return Err(e.into());
        }
        Ok(agent)
    }

    async fn undo(&self, u: &Undo<'_>, stored: Option<&AgentId>) {
        if let Some(id) = stored {
            let _ = self.store.remove_agent(id).await;
        }
        let _ = std::fs::remove_dir_all(u.settings_dir);
        let _ = git::remove(u.repo, u.wt).await;
        let _ = std::fs::remove_dir_all(u.workdir);
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

    pub async fn kill(&self, id: &AgentId) -> Result<(), ServiceError> {
        let a = self.find(id).await?;
        if self.tmux.has_session(&a.tmux_session).await? {
            self.tmux.kill(&a.tmux_session).await?;
        }
        for w in &a.workspaces {
            let repo = self.repo(&w.repo).ok_or_else(|| ServiceError::UnknownRepo(w.repo.clone()))?;
            git::remove(&repo.path, &Worktree { path: PathBuf::from(&w.worktree), branch: w.branch.clone(), base: w.base.clone() }).await?;
        }
        let _ = std::fs::remove_dir_all(&a.workdir);
        let _ = std::fs::remove_dir_all(self.agent_dir(&id.0));
        self.store.remove_agent(id).await?;
        Ok(())
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
                eprintln!("reconcile {}: {e}", a.id.0);
            }
        }
        Ok(())
    }

    async fn reconcile_one(&self, a: &Agent, session_alive: bool, now: i64) -> Result<(), ServiceError> {
        if !session_alive {
            self.store
                .update_agent(&a.id, |x| {
                    if x.status != Status::Dead {
                        x.status = Status::Dead;
                        x.activity = "session lost".into();
                        x.blocked_reason = None;
                        x.status_since = now;
                    }
                })
                .await?;
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
                    eprintln!("mark_unverified {}: {e}", a.id.0);
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
