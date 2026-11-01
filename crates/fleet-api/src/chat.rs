use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// One entry of an agent's chat. `seq` rises by one per item in a session's stream; an updated call keeps its `seq`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct ChatItem {
    #[ts(type = "number")]
    pub seq: u64,
    /// Unix seconds, from the transcript record.
    #[ts(type = "number")]
    pub at: i64,
    pub body: ChatBody,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum ChatBody {
    Prompt {
        text: String,
        from: PromptFrom,
    },
    Reply {
        text: String,
    },
    Call {
        /// Claude's `tool_use` id.
        id: String,
        tool: String,
        /// What the call acts on: a path, a command, a pattern.
        target: String,
        status: CallStatus,
        /// One short line about the outcome, such as `12 lines`.
        result: Option<String>,
        /// A failed call's output, cut to a few lines.
        output: Option<String>,
    },
    Marker {
        kind: MarkerKind,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PromptFrom {
    Owner,
    System,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CallStatus {
    Running,
    Done,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MarkerKind {
    NewSession,
    Compacted,
    SessionEnded,
}

/// A page of chat, oldest first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct ChatPage {
    pub items: Vec<ChatItem>,
    /// Blocks before the first item; a run of consecutive calls counts as one block.
    #[ts(type = "number")]
    pub earlier: u64,
}
