use crate::{AgentKind, BlockedReason, Status, StopCause};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct AgentId(pub String);

/// Who issues a command. Always `"local"` in v1; carried so permissions can be added later.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct Actor(pub String);

/// One repo checked out for one task: a worktree on the task's branch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct Workspace {
    pub repo: String,
    pub branch: String,
    pub worktree: String,
    /// The commit the branch started from.
    pub base: String,
    /// The branch the start commit was on (None: detached, owned, or made before this field existed).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub from_branch: Option<String>,
    pub has_changes: bool,
    /// True when the daemon created this repo (an empty `git init`), so archiving must never delete it.
    #[serde(default)]
    pub owned: bool,
    /// True when the task's branch has commits beyond `base`, as the merge watcher last saw it: only a changed repo
    /// needs a merged PR for the task to be done. Never set for a repo with no URL.
    #[serde(default)]
    #[ts(as = "Option<bool>", optional)]
    pub changed: bool,
}

/// An agent's ask for a repo it has no access to yet; the owner grants or refuses it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct RepoRequest {
    pub repo: String,
    pub why: String,
}

/// One agent session, which runs one task (an idea). The task's title is `task`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct Agent {
    pub id: AgentId,
    /// First line of the prompt, capped.
    pub goal: String,
    /// The task directory: `claude`'s cwd, holding one directory per workspace.
    pub workdir: String,
    /// Exactly one in v1: the starting repo, first in the list.
    pub workspaces: Vec<Workspace>,
    /// The name of the agent's terminal session in the daemon.
    pub session: String,
    pub status: Status,
    pub activity: String,
    #[ts(type = "number")]
    pub status_since: i64,
    /// True when any workspace has changes.
    pub has_changes: bool,
    pub paused: bool,
    /// Who runs this agent. Always the local user for now; kept so a team hub can be added later.
    pub owner: String,
    /// Set only while `status` is `NeedsYou`.
    pub blocked_reason: Option<BlockedReason>,
    /// Set while `blocked_reason` is `RepoAccess`.
    #[serde(default)]
    pub repo_request: Option<RepoRequest>,
    /// Set while `status` is `Stopped`.
    #[serde(default)]
    pub stop_cause: Option<StopCause>,
    /// Size of the task directory, measured when the agent stops; None until then.
    #[serde(default)]
    #[ts(type = "number | null")]
    pub disk_bytes: Option<u64>,
    /// The persona (Claude and GitHub login) the task runs under; empty on agents that predate personas.
    #[serde(default)]
    pub persona: String,
    /// The product the task belongs to, set at spawn and never changed.
    #[serde(default)]
    pub product: String,
    #[serde(default)]
    #[ts(as = "Option<AgentKind>", optional)]
    pub kind: AgentKind,
    /// The id of this agent's task; None means this agent's own id.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub task: Option<AgentId>,
    /// The orchestrator type or the specialist name.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub role: Option<String>,
    /// The breakdown step a specialist works on.
    #[serde(default)]
    #[ts(type = "number | null", optional)]
    pub step: Option<u32>,
    /// The model this agent was started on (`claude --model`); None for a solo agent or when Claude's own default
    /// applies.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub model: Option<String>,
    /// The argv that attaches a terminal to this agent's tmux session. Filled by the daemon when it serves the agent,
    /// never stored; None while `status` is `Stopped`.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub attach: Option<Vec<String>>,
}
