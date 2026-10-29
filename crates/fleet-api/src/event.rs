use crate::{Agent, AgentId, BlockedReason, RepoRequest, Status, StopCause, Workspace};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
// Events are rare and short-lived; boxing `Agent` would ripple through every consumer for no measurable gain.
#[allow(clippy::large_enum_variant)]
#[ts(export)]
pub enum EventKind {
    AgentCreated {
        agent: Agent,
    },
    StatusChanged {
        id: AgentId,
        status: Status,
        activity: String,
        #[ts(type = "number")]
        since: i64,
        has_changes: bool,
        paused: bool,
        blocked_reason: Option<BlockedReason>,
        /// Set while `blocked_reason` is `RepoAccess`.
        #[serde(default)]
        #[ts(optional = nullable)]
        repo_request: Option<RepoRequest>,
        #[serde(default)]
        #[ts(optional = nullable)]
        stop_cause: Option<StopCause>,
        #[serde(default)]
        #[ts(type = "number | null", optional)]
        disk_bytes: Option<u64>,
    },
    /// A workspace was added, or one's `has_changes` flipped.
    WorkspacesChanged {
        id: AgentId,
        workspaces: Vec<Workspace>,
    },
    /// The agent named its task (`set_title`).
    TitleChanged {
        id: AgentId,
        title: String,
    },
    AgentRemoved {
        id: AgentId,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct Event {
    #[ts(type = "number")]
    pub seq: u64,
    pub kind: EventKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct Snapshot {
    #[ts(type = "number")]
    pub seq: u64,
    pub agents: Vec<Agent>,
}
