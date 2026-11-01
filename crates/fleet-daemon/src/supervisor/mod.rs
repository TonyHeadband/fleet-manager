//! A persona's supervisor: one agent of kind `Supervisor` per conversation, with no task and no product, running in
//! the persona's own folder.
mod files;

pub use files::folder;

use crate::agents::{
    fleet_config, one_line, write_prompt, AgentService, Clock, SpawnError, Start, MAX_PROMPT_BYTES,
    PROMPT_FILE, SESSION_PREFIX,
};
use crate::fleet::ops::agent_vars;
use crate::hooks_cfg::{write_mcp_config, write_settings};
use crate::repo_rules::RepoRules;
use crate::store::StoreError;
use fleet_api::{
    Agent, AgentId, AgentKind, Conversation, Effort, PermissionMode, Status, TaskMark, ToolId,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;
use tracing::{info, warn};

const MODE: PermissionMode = PermissionMode::Auto;
const EFFORT: Effort = Effort::High;
/// The subagent types a supervisor may start: both only read.
const SUBAGENT_TYPES: [&str; 2] = ["Explore", "Plan"];
const SUBAGENT_REFUSAL: &str =
    "the supervisor's subagents are read-only (Explore, Plan); start a task to change anything";

#[derive(Debug, thiserror::Error)]
pub enum SupervisorError {
    #[error("the message is empty")]
    EmptyText,
    #[error(transparent)]
    Spawn(#[from] SpawnError),
}

impl From<StoreError> for SupervisorError {
    fn from(e: StoreError) -> Self {
        SupervisorError::Spawn(e.into())
    }
}

pub struct Supervisor {
    svc: Arc<AgentService>,
    clock: Arc<dyn Clock>,
    /// Held while a new conversation replaces the persona folder's PROMPT.md, so two starts never interleave there.
    prompt_lock: tokio::sync::Mutex<()>,
}

impl Supervisor {
    pub fn new(svc: Arc<AgentService>) -> Supervisor {
        let clock = svc.options().clock.clone();
        Supervisor {
            svc,
            clock,
            prompt_lock: tokio::sync::Mutex::new(()),
        }
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
        let cwd = folder(svc.config(), persona);
        let settings_dir = svc.agent_dir(&id.0);
        std::fs::create_dir_all(&cwd).map_err(SpawnError::from)?;
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
                remove_dir(&settings_dir);
                return Err(SpawnError::from(e).into());
            }
        };
        let files = {
            let _prompt = self.prompt_lock.lock().await;
            files::write_claude_md(&cwd).and_then(|()| {
                // The folder is every conversation's, so the last one's prompt is replaced.
                match std::fs::remove_file(cwd.join(PROMPT_FILE)) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
                    _ => {}
                }
                write_prompt(&cwd, text)
            })
        };
        if let Err(e) = files {
            remove_dir(&settings_dir);
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
            remove_dir(&settings_dir);
            return Err(e.into());
        }
        if let Err(e) = svc.store.insert_conversation(&id, &env.id, now).await {
            self.forget(&id, &settings_dir).await;
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
            // The conversation row stays, without an agent: `list` leaves it out.
            self.forget(&id, &settings_dir).await;
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

    /// Undoes a start whose agent was stored: its card and its settings go.
    async fn forget(&self, id: &AgentId, settings_dir: &std::path::Path) {
        if let Err(e) = self.svc.store.remove_agent(id).await {
            warn!(agent_id = %id.0, error = %one_line(&e.to_string()), "cannot remove a failed supervisor start");
        }
        remove_dir(settings_dir);
    }
}

fn remove_dir(dir: &std::path::Path) {
    match std::fs::remove_dir_all(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            warn!(dir = %dir.display(), error = %e, "cannot remove a failed supervisor start's settings")
        }
        _ => {}
    }
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
