//! A persona's supervisor: one agent of kind `Supervisor` per conversation, with no task and no product, each running
//! in its own folder in the persona's.
mod events;
mod files;
pub(crate) mod tools;

pub(crate) use events::route_with;
pub use events::{route, start_event_watcher};
pub use files::{conversation_folder, folder};

use crate::agents::{
    fleet_config, one_line, write_prompt, AgentService, Clock, ServiceError, SpawnError, Start,
    MAX_PROMPT_BYTES, SESSION_PREFIX,
};
use crate::first_run::trust_folder;
use crate::fleet::ops::agent_vars;
use crate::hooks_cfg::{write_mcp_config, write_settings};
use crate::repo_rules::RepoRules;
use crate::store::StoreError;
use fleet_api::{
    Agent, AgentId, AgentKind, Conversation, Effort, PermissionMode, Status, TaskMark, ToolId,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::{info, warn};

const MODE: PermissionMode = PermissionMode::Auto;
const EFFORT: Effort = Effort::High;
/// The subagent types a supervisor may start: both only read.
const SUBAGENT_TYPES: [&str; 2] = ["Explore", "Plan"];
const SUBAGENT_REFUSAL: &str =
    "the supervisor's subagents are read-only (Explore, Plan); start a task to change anything";
/// How long a conversation sits idle in Review before the reaper ends its process.
pub const IDLE_EXIT: Duration = Duration::from_secs(600);
/// How often the reaper looks.
pub const REAP_EVERY: Duration = Duration::from_secs(60);
/// How long a message waits for the TUI to turn bracketed paste on, the sign its input box is up, before it is typed
/// anyway: a terminal that never reports it must not lose the message.
const READY_WAIT: Duration = Duration::from_secs(15);

/// One lock per conversation, held while a message resumes it and types, and while the reaper ends it: messages are
/// typed in the order they came, and never into a process being resumed or ended. Process-wide, so every
/// `Supervisor` and the reaper share it.
static TURNS: LazyLock<Mutex<HashMap<AgentId, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(Default::default);

fn turn(id: &AgentId) -> Arc<tokio::sync::Mutex<()>> {
    TURNS
        .lock()
        .expect("turns poisoned")
        .entry(id.clone())
        .or_default()
        .clone()
}

#[derive(Debug, thiserror::Error)]
pub enum SupervisorError {
    #[error("the message is empty")]
    EmptyText,
    #[error("no such conversation")]
    NotFound,
    #[error("cannot resume the conversation: {0}")]
    Resume(String),
    #[error(transparent)]
    Spawn(#[from] SpawnError),
    #[error(transparent)]
    Service(#[from] ServiceError),
}

impl From<StoreError> for SupervisorError {
    fn from(e: StoreError) -> Self {
        SupervisorError::Spawn(e.into())
    }
}

pub struct Supervisor {
    svc: Arc<AgentService>,
    clock: Arc<dyn Clock>,
}

impl Supervisor {
    pub fn new(svc: Arc<AgentService>) -> Supervisor {
        let clock = svc.options().clock.clone();
        Supervisor { svc, clock }
    }

    /// Creates the conversation row and the agent, writes `text` as PROMPT.md, starts with `Start::New`.
    pub async fn new_conversation(
        &self,
        persona: &str,
        text: &str,
    ) -> Result<AgentId, SupervisorError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(SupervisorError::EmptyText);
        }
        if text.len() > MAX_PROMPT_BYTES {
            return Err(SpawnError::PromptTooLong.into());
        }
        let svc = &self.svc;
        svc.options()
            .tools
            .require(ToolId::Claude)
            .await
            .map_err(SpawnError::from)?;
        if persona.is_empty() {
            return Err(SpawnError::NoPersona.into());
        }
        // Before the folder: it is named after the persona, which must be a real one.
        let env = svc
            .personas()
            .ready_env(persona)
            .await
            .map_err(SpawnError::from)?;
        let fleet =
            fleet_config(&svc.config().agent_config_dir).map_err(SpawnError::AgentConfig)?;
        let role = files::role_settings(svc, persona).await?;

        let id = AgentId(uuid::Uuid::new_v4().simple().to_string());
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let session = format!("{SESSION_PREFIX}{}", &id.0[..8]);
        let cwd = conversation_folder(svc.config(), persona, &id);
        let settings_dir = svc.agent_dir(&id.0);
        std::fs::create_dir_all(&cwd).map_err(SpawnError::from)?;
        // Task folders inherit the trust the persona gave the tasks root at login; this folder has none of its own.
        if let Some(claude_dir) = env.claude_dir() {
            let dir = cwd.clone();
            let trusted = tokio::task::spawn_blocking(move || trust_folder(&claude_dir, &dir))
                .await
                .map_err(std::io::Error::other)
                .and_then(|r| r);
            if let Err(e) = trusted {
                remove_dirs(&[&cwd]);
                return Err(SpawnError::from(e).into());
            }
        }
        let written = write_settings(
            &settings_dir,
            &svc.options().base_url,
            &id,
            &RepoRules::default(),
            &fleet.settings,
            &BTreeMap::new(),
            &cwd,
            &role,
        )
        .and_then(|s| {
            Ok((
                s,
                write_mcp_config(&settings_dir, &svc.options().base_url, &id, &token)?,
            ))
        });
        let (settings, mcp) = match written {
            Ok(p) => p,
            Err(e) => {
                remove_dirs(&[&settings_dir, &cwd]);
                return Err(SpawnError::from(e).into());
            }
        };
        if let Err(e) = files::write_claude_md(&cwd).and_then(|()| write_prompt(&cwd, text)) {
            remove_dirs(&[&settings_dir, &cwd]);
            return Err(SpawnError::from(e).into());
        }

        let now = self.clock.now();
        let agent = Agent {
            id: id.clone(),
            goal: text
                .lines()
                .next()
                .unwrap_or(text)
                .chars()
                .take(120)
                .collect(),
            title: None,
            workdir: cwd.to_string_lossy().into_owned(),
            workspaces: vec![],
            session: session.clone(),
            status: Status::Queued,
            activity: "starting".into(),
            status_since: now,
            has_changes: false,
            paused: false,
            owner: svc.options().owner.clone(),
            blocked_reason: None,
            repo_request: None,
            stop_cause: None,
            disk_bytes: None,
            persona: env.id.clone(),
            product: String::new(),
            kind: AgentKind::Supervisor,
            task: None,
            role: None,
            step: None,
            model: None,
            mode: Some(MODE),
            effort: Some(EFFORT),
            attach: None,
        };
        // Stored before the session starts: SessionStart can arrive within milliseconds of the process starting.
        if let Err(e) = svc.store.insert_agent(&agent, &token).await {
            remove_dirs(&[&settings_dir, &cwd]);
            return Err(e.into());
        }
        if let Err(e) = svc.store.insert_conversation(&id, &env.id, now).await {
            self.forget(&id, &[&settings_dir, &cwd]).await;
            return Err(e.into());
        }

        env.seed_first_run().await;
        let vars = agent_vars(&env, AgentKind::Supervisor, &[]);
        let launched = svc
            .launch(
                &id,
                &session,
                &cwd,
                &settings,
                &mcp,
                &token,
                &vars,
                None,
                MODE,
                EFFORT,
                Start::New,
            )
            .await;
        if let Err(e) = launched {
            self.forget(&id, &[&settings_dir, &cwd]).await;
            return Err(e.into());
        }
        info!(agent_id = %id.0, persona = %env.id, "supervisor conversation started");
        Ok(id)
    }

    /// The persona's conversations, most recently used first, each with the tasks it started.
    pub async fn list(&self, persona: &str) -> Result<Vec<Conversation>, SupervisorError> {
        let store = &self.svc.store;
        let rows = store.conversations(persona).await?;
        let agents = store.snapshot().await?.agents;
        let tasks = store.tasks().await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let a = agents.iter().find(|a| a.id == row.id)?;
                let marks = tasks
                    .iter()
                    .filter(|t| t.origin.as_ref() == Some(&row.id))
                    .map(|t| TaskMark {
                        id: t.id.clone(),
                        title: t.title.clone().unwrap_or_else(|| t.goal.clone()),
                        status: agents
                            .iter()
                            .find(|l| &l.id == t.lead.as_ref().unwrap_or(&t.id))
                            .map_or(Status::Stopped, |l| l.status),
                    })
                    .collect();
                Some(Conversation {
                    id: row.id,
                    persona: row.persona,
                    title: a.title.clone(),
                    created_at: row.created_at,
                    last_used_at: row.last_used_at,
                    live: a.status != Status::Stopped,
                    tasks: marks,
                })
            })
            .collect())
    }

    /// Types `text` into the conversation `id` once its TUI is ready, resuming its process first when it has exited.
    /// Messages to one conversation are typed in the order they came.
    pub async fn message(&self, id: &AgentId, text: &str) -> Result<(), SupervisorError> {
        self.send(id, text, false).await.map(|_| ())
    }

    /// `message`, unless a turn runs or is pending once its turn lock is held (`turn_pending`): then nothing is typed
    /// and it returns false. For the daemon's notices, which must never land in a running turn.
    pub async fn message_if_idle(&self, id: &AgentId, text: &str) -> Result<bool, SupervisorError> {
        self.send(id, text, true).await
    }

    async fn send(&self, id: &AgentId, text: &str, if_idle: bool) -> Result<bool, SupervisorError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(SupervisorError::EmptyText);
        }
        self.conversation(id).await?;
        let turn = turn(id);
        let _turn = turn.lock().await;
        // Read again under the lock: the message before this one may have resumed it, or the reaper ended it.
        let a = self.conversation(id).await?;
        if if_idle && self.turn_pending(&a).await? {
            return Ok(false);
        }
        if a.status == Status::Stopped {
            self.svc
                .resume_stopped(&a)
                .await
                .map_err(SupervisorError::Resume)?;
        }
        self.wait_ready(&a.session).await;
        self.svc.answer(id, text).await?;
        self.svc
            .store
            .touch_conversation(id, self.clock.now())
            .await?;
        info!(agent_id = %id.0, persona = %a.persona, resumed = a.status == Status::Stopped, "supervisor message sent");
        Ok(true)
    }

    /// Whether a turn runs or is about to: working, a prompt up, or a message submitted no earlier than its last
    /// status change, whose turn Claude's hook has yet to report (the clock has whole seconds, so a tie counts as
    /// pending). An exited conversation runs no turn; `send` resumes it.
    async fn turn_pending(&self, a: &Agent) -> Result<bool, SupervisorError> {
        if matches!(a.status, Status::Working | Status::NeedsYou) {
            return Ok(true);
        }
        if a.status == Status::Stopped {
            return Ok(false);
        }
        let touched = self
            .svc
            .store
            .conversations(&a.persona)
            .await?
            .into_iter()
            .find(|r| r.id == a.id)
            .map(|r| r.last_used_at);
        Ok(touched.is_some_and(|at| at >= a.status_since))
    }

    async fn conversation(&self, id: &AgentId) -> Result<Agent, SupervisorError> {
        match self.svc.agent(id).await {
            Ok(a) if a.kind == AgentKind::Supervisor => Ok(a),
            Ok(_) | Err(ServiceError::NotFound) => Err(SupervisorError::NotFound),
            Err(e) => Err(e.into()),
        }
    }

    /// Waits until the session has turned bracketed paste on, as Claude's TUI does once its input box is up, so a
    /// resumed conversation's message is not typed into its starting screen. Gives up after `READY_WAIT`.
    async fn wait_ready(&self, session: &str) {
        let end = Instant::now() + READY_WAIT;
        loop {
            match self.svc.sessions.bracketed_paste(session) {
                Ok(true) => return,
                // Gone: `answer` says so.
                Err(_) => return,
                Ok(false) if Instant::now() >= end => {
                    warn!(
                        session,
                        "the supervisor's TUI never reported ready; typing anyway"
                    );
                    return;
                }
                Ok(false) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
    }

    /// Undoes a start whose agent was stored: its card, its conversation row and its folders go.
    async fn forget(&self, id: &AgentId, dirs: &[&Path]) {
        if let Err(e) = self.svc.store.remove_agent(id).await {
            warn!(agent_id = %id.0, error = %one_line(&e.to_string()), "cannot remove a failed supervisor start");
        }
        if let Err(e) = self.svc.store.remove_conversation(id).await {
            warn!(agent_id = %id.0, error = %one_line(&e.to_string()), "cannot remove a failed supervisor start's conversation");
        }
        remove_dirs(dirs);
    }
}

fn remove_dirs(dirs: &[&Path]) {
    for dir in dirs {
        match std::fs::remove_dir_all(dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                warn!(dir = %dir.display(), error = %e, "cannot remove a failed supervisor start's folder")
            }
            _ => {}
        }
    }
}

/// Every `every`, ends the process of each conversation that has sat in Review for `IDLE_EXIT`; a message resumes
/// it. A conversation that is working, needs you, or is starting is never ended, nor one that got a message within
/// `IDLE_EXIT`: it is still in Review until Claude's hook reports the message. Each round also puts each room ask
/// left open for 30 minutes to the owner once more.
pub fn start_idle_reaper(svc: Arc<AgentService>, every: Duration) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(every);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            if let Err(e) = reap(&svc).await {
                warn!(error = %one_line(&e.to_string()), "the supervisor idle reaper could not read the agents");
            }
            if let Err(e) = crate::rooms::repost_stale_asks(&svc).await {
                warn!(error = %one_line(&e.to_string()), "cannot repost the stale room asks");
            }
        }
    })
}

async fn reap(svc: &AgentService) -> Result<(), StoreError> {
    let idle = |a: &Agent, now: i64| {
        a.kind == AgentKind::Supervisor
            && a.status == Status::Review
            && now - a.status_since >= IDLE_EXIT.as_secs() as i64
    };
    let clock = svc.options().clock.clone();
    for a in svc.store.snapshot().await?.agents {
        if !idle(&a, clock.now()) {
            continue;
        }
        // A message on its way resumes or types into it: left for the next round.
        let turn = turn(&a.id);
        let Ok(_turn) = turn.try_lock() else {
            continue;
        };
        match svc.agent(&a.id).await {
            Ok(now) if idle(&now, clock.now()) => {}
            _ => continue,
        }
        let rows = svc.store.conversations(&a.persona).await?;
        let used = rows.iter().find(|r| r.id == a.id).map(|r| r.last_used_at);
        if used.is_some_and(|at| clock.now() - at < IDLE_EXIT.as_secs() as i64) {
            continue;
        }
        match svc.stop(&a.id).await {
            Ok(()) => {
                info!(agent_id = %a.id.0, persona = %a.persona, "idle supervisor conversation ended")
            }
            Err(e) => {
                warn!(agent_id = %a.id.0, error = %one_line(&e.to_string()), "cannot end an idle supervisor conversation")
            }
        }
    }
    Ok(())
}

/// The `PreToolUse` answer that refuses a subagent other than Explore or Plan, for a supervisor's hook; `None` for
/// any other tool call. The caller checks the agent is a supervisor: no other agent is refused here.
pub fn subagent_refusal(body: &Value) -> Option<Value> {
    let tool = body.get("tool_name")?.as_str()?;
    if !matches!(tool, "Agent" | "Task") {
        return None;
    }
    let kind = body
        .get("tool_input")
        .and_then(|i| i.get("subagent_type"))
        .and_then(Value::as_str);
    if kind.is_some_and(|k| SUBAGENT_TYPES.contains(&k)) {
        return None;
    }
    Some(json!({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "deny",
        "permissionDecisionReason": SUBAGENT_REFUSAL,
    }}))
}
