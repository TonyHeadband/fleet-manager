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

/// What a post costs its recipient: see `Post`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PostKind {
    /// History only: wakes nobody.
    Announcement,
    /// Wakes its recipients; the sender carries on.
    Message,
    /// Wakes its recipient and holds the sender until it is answered.
    Ask,
    /// The one answer to an ask (`reply_to`); the first wins.
    Answer,
}

/// One post in a room. It carries its sender and is never an instruction from the owner.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct Post {
    #[ts(type = "number")]
    pub id: i64,
    pub room: String,
    pub sender: MemberRef,
    /// The sender as people read it: the task's title, `supervisor`, `owner` or `Fleet Manager`.
    pub sender_name: String,
    /// One member, or None for everyone in the room.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub to: Option<MemberRef>,
    pub kind: PostKind,
    /// The ask an answer answers.
    #[serde(default)]
    #[ts(type = "number | null", optional)]
    pub reply_to: Option<i64>,
    pub text: String,
    #[ts(type = "number")]
    pub at: i64,
    /// Set on an ask once it has its answer.
    #[serde(default)]
    #[ts(type = "number | null", optional)]
    pub answered_at: Option<i64>,
    /// The breakdown version an ask asks to approve.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub breakdown_version: Option<u32>,
}

/// The owner's post from the Rooms widget: an announcement or a message. Asks are the orchestrators'.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct NewPost {
    pub kind: PostKind,
    /// One member of the room, or None for everyone.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub to: Option<MemberRef>,
    pub text: String,
}

/// The owner's answer to an ask. `approve: true` answers `approve`, which approves a breakdown's version; otherwise
/// the answer is `text`, which for a breakdown's ask requests changes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct AskReply {
    #[serde(default)]
    #[ts(optional = nullable)]
    pub text: Option<String>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub approve: Option<bool>,
}
