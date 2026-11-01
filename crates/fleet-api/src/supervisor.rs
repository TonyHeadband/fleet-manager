use crate::{AgentId, Status};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// One of a persona's supervisor conversations; each is an agent of kind `Supervisor`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct Conversation {
    pub id: AgentId,
    pub persona: String,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub title: Option<String>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub last_used_at: i64,
    /// Whether its process is running (it exits when idle and resumes on the next message).
    pub live: bool,
    /// The tasks it started.
    pub tasks: Vec<TaskMark>,
}

/// A task a conversation started, as a mark in its header.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct TaskMark {
    pub id: AgentId,
    pub title: String,
    pub status: Status,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct NewConversation {
    pub persona: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct ConversationMessage {
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TaskEventKind {
    NeedsYou,
    ChecksPassed,
    ChecksFailed,
    PrOpened,
    PrMerged,
}

/// Something that happened to a task, kept for the conversation that started it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct TaskEvent {
    #[ts(type = "number")]
    pub id: i64,
    /// `None` when the persona had no conversation: the event is only toasted.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub conversation: Option<AgentId>,
    pub persona: String,
    pub task: AgentId,
    pub task_title: String,
    pub kind: TaskEventKind,
    pub text: String,
    #[ts(type = "number")]
    pub at: i64,
}
