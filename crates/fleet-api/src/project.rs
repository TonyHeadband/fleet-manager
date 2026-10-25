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

/// One thing an archive touches or leaves, measured when the plan was made.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct PlanItem {
    pub path: String,
    #[ts(type = "number")]
    pub bytes: u64,
    /// Changed and untracked files in a worktree: lost on archive, since only commits survive on the branch.
    pub uncommitted_files: u32,
    /// True when this item's contents are kept, not removed: the project folder's remaining contents, or a repository
    /// found where a worktree was. Not counted in `ArchivePlan::total`.
    pub stays: bool,
}

/// What archiving would do, for the user to confirm: each worktree, the daemon's files, and last the project folder
/// itself, whose `bytes` are what else it holds. Those stay, and so does the folder unless it ends up empty.
/// `total` is what archiving frees: every item whose `stays` is false.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct ArchivePlan {
    pub items: Vec<PlanItem>,
    #[ts(type = "number")]
    pub total: u64,
}

/// The daemon's error text (with a 409) when an archive's confirmed plan no longer matches what is on disk. Clients
/// match it to fetch and show a fresh plan instead of reporting a failure.
pub const PLAN_CHANGED: &str = "the project changed since you confirmed; review the new plan";

/// A size in SI units with one decimal ("7.6 GB", "12.0 KB"); plain bytes below a kilobyte. SI, not binary, so the
/// figure matches what `df -H` and file managers print.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    while value >= 999.95 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
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
    #[serde(default)]
    pub persona: String,
}

/// What an orphan is: something the daemon made, or a tool left, that no project record owns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum OrphanKind {
    /// A folder directly under the projects root, with the worktrees registered inside it.
    ProjectFolder,
    /// A folder under the daemon's `agents/` directory.
    AgentFolder,
    /// A worktree under the projects root outside any project folder, possibly with its folder gone.
    Worktree,
    /// A live tmux session with the daemon's prefix that no agent owns. Ended, never archived.
    Session,
}

/// Something found with no project record. `id` is `"<kind>:<path or session name>"`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct Orphan {
    pub id: String,
    pub kind: OrphanKind,
    /// The folder or worktree path, or the session name.
    pub path: String,
    /// The registered worktrees inside a project folder.
    pub worktrees: Vec<String>,
    /// None until the background measurer has weighed the folder, and for sessions.
    #[serde(default)]
    #[ts(type = "number | null")]
    pub bytes: Option<u64>,
}

/// Everything the Projects view shows: the records, what was found with no record, and the clone cache's size.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct ProjectList {
    pub projects: Vec<Project>,
    pub orphans: Vec<Orphan>,
    /// The size of the daemon's clone cache (`<state_dir>/repos`); None until the background measurer has walked it.
    #[serde(default)]
    #[ts(type = "number | null")]
    pub clones_bytes: Option<u64>,
}
