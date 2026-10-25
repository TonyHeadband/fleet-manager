use crate::{AgentId, StopCause, Workspace};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// Where a project is in its life: its agent is alive, its agent has stopped, or its folder has been archived.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ProjectState {
    Running,
    Stopped,
    Archived,
}

/// One line of a project's history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct LogEntry {
    #[ts(type = "number")]
    pub at: i64,
    pub text: String,
}

/// Seconds the project's agents spent in each status. Talking and Stopped are not counted.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct StatusTimes {
    #[ts(type = "number")]
    pub queued: i64,
    #[ts(type = "number")]
    pub working: i64,
    #[ts(type = "number")]
    pub needs_you: i64,
    #[ts(type = "number")]
    pub review: i64,
}

/// What one part of a project's folder weighs on disk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct FootprintItem {
    pub label: String,
    pub path: String,
    #[ts(type = "number")]
    pub bytes: u64,
    /// The part of `bytes` that is regenerable build output, when the repo has any.
    #[serde(default)]
    #[ts(type = "number | null")]
    pub build_bytes: Option<u64>,
}

/// The project folder's size, measured at `measured_at`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct Footprint {
    #[ts(type = "number")]
    pub measured_at: i64,
    pub items: Vec<FootprintItem>,
    #[ts(type = "number")]
    pub total: u64,
}

/// What archiving a project removed and kept.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct ArchiveReport {
    pub removed: Vec<String>,
    #[ts(type = "number")]
    pub freed: u64,
    #[serde(default)]
    pub kept: Option<String>,
}

/// The record of a project, kept after its agent is gone. `id` is the id of the agent that started it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct Project {
    pub id: AgentId,
    /// The project folder's basename.
    pub name: String,
    pub task: String,
    pub workdir: String,
    pub repos: Vec<Workspace>,
    pub state: ProjectState,
    #[serde(default)]
    pub stop_cause: Option<StopCause>,
    /// None for records made on upgrade, when the start time is unknown.
    #[serde(default)]
    #[ts(type = "number | null")]
    pub created_at: Option<i64>,
    #[serde(default)]
    #[ts(type = "number | null")]
    pub stopped_at: Option<i64>,
    #[serde(default)]
    #[ts(type = "number | null")]
    pub archived_at: Option<i64>,
    pub agents_used: u32,
    pub times: StatusTimes,
    pub log: Vec<LogEntry>,
    #[serde(default)]
    pub footprint: Option<Footprint>,
    #[serde(default)]
    pub last_screen: Option<String>,
    #[serde(default)]
    pub archive: Option<ArchiveReport>,
}
