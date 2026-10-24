use crate::Actor;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// The repos the daemon is configured to run agents in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct RepoList {
    pub repos: Vec<String>,
}

/// What one workspace has changed since its branch started, as unified diff text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct WorkspaceDiff {
    pub repo: String,
    pub diff: String,
    /// True when `diff` was cut at the size cap.
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct DiffReport {
    pub workspaces: Vec<WorkspaceDiff>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct SpawnRequest {
    pub actor: Actor,
    /// The repo the project starts in.
    pub repo: String,
    pub prompt: String,
    pub task_slug: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct AnswerRequest {
    pub actor: Actor,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct ControlRequest {
    pub actor: Actor,
}
