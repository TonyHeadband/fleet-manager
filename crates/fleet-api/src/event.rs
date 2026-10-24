use crate::{Agent, AgentId, BlockedReason, Status, Workspace};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum EventKind {
    AgentCreated { agent: Agent },
    StatusChanged {
        id: AgentId,
        status: Status,
        activity: String,
        #[ts(type = "number")]
        since: i64,
        has_changes: bool,
        paused: bool,
        blocked_reason: Option<BlockedReason>,
    },
    /// A workspace was added, or one's `has_changes` flipped.
    WorkspacesChanged { id: AgentId, workspaces: Vec<Workspace> },
    AgentRemoved { id: AgentId },
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
