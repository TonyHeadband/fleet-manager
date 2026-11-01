use crate::AgentId;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RoomKind {
    /// Where the orchestrators of one supervisor conversation's tasks, and that conversation, talk.
    Operation,
    /// Where every orchestrator of a product talks.
    Product,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct Room {
    pub id: String,
    pub kind: RoomKind,
    /// The product's name, or the conversation's title (`Untitled` when it has none).
    pub title: String,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub product: Option<String>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub conversation: Option<AgentId>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub persona: Option<String>,
    #[serde(default)]
    #[ts(type = "number | null", optional)]
    pub archived_at: Option<i64>,
    /// Posts the owner has not read.
    pub unread: u32,
}

/// Who is in a room or sent a post.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MemberRef {
    Owner,
    Agent(AgentId),
    /// The daemon's own announcements.
    Daemon,
}

impl MemberRef {
    /// The text the store keeps: `owner`, `daemon` or the agent id.
    pub fn key(&self) -> &str {
        match self {
            MemberRef::Owner => "owner",
            MemberRef::Daemon => "daemon",
            MemberRef::Agent(id) => &id.0,
        }
    }

    pub fn from_key(key: &str) -> MemberRef {
        match key {
            "owner" => MemberRef::Owner,
            "daemon" => MemberRef::Daemon,
            id => MemberRef::Agent(AgentId(id.to_string())),
        }
    }
}
