use crate::{AgentId, TaskEventKind};
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
    /// Something that happened to a task the supervisor started, merged into its conversation's chat.
    Event {
        task: AgentId,
        task_title: String,
        kind: TaskEventKind,
        text: String,
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

/// A prompt the agent waits on, as the chat shows it. Sent as the chat stream's `ask` event (`null` when none).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct ChatAsk {
    pub id: String,
    pub kind: AskKind,
    /// The label of the choice that answered it; the ask is sent once answered before it clears.
    pub answered: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum AskKind {
    /// Claude wants to run a tool. Options: `allow`, `allow_session` (only where Claude suggests rules), `deny`.
    Permission {
        tool: String,
        #[schema(value_type = Object)]
        #[ts(type = "unknown")]
        input: serde_json::Value,
        options: Vec<AskOption>,
    },
    /// AskUserQuestion. Option ids are `<question>.<option>`, both from 0.
    Question { questions: Vec<AskQuestion> },
    /// A prompt with no hook held for it: the agent's current screen, answered through the reply box.
    Screen { text: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct AskOption {
    pub id: String,
    pub label: String,
    /// What Claude says about a question's option; None for a permission's.
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct AskQuestion {
    pub header: String,
    pub question: String,
    pub options: Vec<AskOption>,
    pub multi_select: bool,
}

/// The body of `POST /api/agents/{id}/asks/{ask}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct AskAnswer {
    /// A permission's option id. For a question, the chosen option ids joined with `,` (several only for a
    /// multi-select or a form of several questions); may be empty when `text` answers.
    pub choice: String,
    /// A question's free-text answer, for the one question no option of `choice` answers.
    pub text: Option<String>,
}
