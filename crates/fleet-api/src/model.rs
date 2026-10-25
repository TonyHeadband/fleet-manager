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

/// One repo checked out for one project: a worktree on the project's branch.
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
}

/// An agent's ask for a repo it has no access to yet; the owner grants or refuses it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct RepoRequest {
    pub repo: String,
    pub why: String,
}

/// One agent session, which runs one project (an idea). The project's title is `task`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct Agent {
    pub id: AgentId,
    /// First line of the prompt, capped.
    pub task: String,
    /// The project directory: `claude`'s cwd, holding one directory per workspace.
    pub workdir: String,
    /// Exactly one in v1: the starting repo, first in the list.
    pub workspaces: Vec<Workspace>,
    pub tmux_session: String,
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
    /// Size of the project directory, measured when the agent stops; None until then.
    #[serde(default)]
    #[ts(type = "number | null")]
    pub disk_bytes: Option<u64>,
    /// The persona (Claude and GitHub login) the project runs under; empty on agents that predate personas.
    #[serde(default)]
    pub persona: String,
    #[serde(default)]
    #[ts(as = "Option<AgentKind>", optional)]
    pub kind: AgentKind,
    /// The project record's id; None means this agent's own id (see `project_id`).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub project: Option<AgentId>,
    /// The orchestrator type or the specialist name.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub role: Option<String>,
    /// The breakdown step a specialist works on.
    #[serde(default)]
    #[ts(type = "number | null", optional)]
    pub step: Option<u32>,
}
