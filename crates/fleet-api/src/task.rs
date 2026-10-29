use crate::{
    AgentId, AgentKind, BreakdownVersion, FleetEvent, GateRun, PrRules, StopCause, Workspace,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// Where a task is in its life: its agent is alive, its agent has stopped, or its folder has been archived.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TaskState {
    Running,
    Stopped,
    Archived,
}

/// One line of a task's history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct LogEntry {
    #[ts(type = "number")]
    pub at: i64,
    pub text: String,
}

/// Seconds the task's agents spent in each status. Talking and Stopped are not counted.
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

/// What one part of a task's folder weighs on disk.
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

/// The task folder's size, measured at `measured_at`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct Footprint {
    #[ts(type = "number")]
    pub measured_at: i64,
    pub items: Vec<FootprintItem>,
    #[ts(type = "number")]
    pub total: u64,
}

/// What archiving a task removed and kept.
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
    /// True when this item's contents are kept, not removed: the task folder's remaining contents, or a repository
    /// found where a worktree was. Not counted in `ArchivePlan::total`.
    pub stays: bool,
}

/// What archiving would do, for the user to confirm: each worktree, the daemon's files, and last the task folder
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
pub const PLAN_CHANGED: &str = "the task changed since you confirmed; review the new plan";

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

/// A pull request's state as GitHub reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PrState {
    Open,
    Merged,
    Closed,
}

/// A PR whose head is the task's branch on one of its repos.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct TaskPr {
    pub url: String,
    pub repo: String,
    pub state: PrState,
    #[ts(type = "number | null")]
    pub merged_at: Option<i64>,
}

/// Where a task stands for its release's count; see `progress::task_outcome`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TaskOutcome {
    Open,
    Merged,
    DoneByHand,
    /// Archived without a merged PR: left out of the release's count.
    Dropped,
}

/// The record of a task, kept after its agent is gone. `id` is the id of the agent that started it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct Task {
    pub id: AgentId,
    /// The product the task belongs to; empty only on an archived orphan's record, which belongs to none.
    pub product: String,
    /// The release of `product` the task is in; None is the product's backlog.
    #[ts(type = "number | null")]
    pub release: Option<i64>,
    /// The task folder's basename.
    pub name: String,
    pub goal: String,
    /// The lead agent's `title`.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub title: Option<String>,
    pub workdir: String,
    pub repos: Vec<Workspace>,
    pub state: TaskState,
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
    #[serde(default)]
    #[ts(as = "Option<AgentKind>", optional)]
    pub kind: AgentKind,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub role: Option<String>,
    /// The specialists the orchestrator may use: every one that loaded when the task started, less the product's
    /// hidden ones. Records from before the roster went away still read it as `roster`.
    #[serde(default, alias = "roster")]
    #[ts(as = "Option<Vec<String>>", optional)]
    pub specialists: Vec<String>,
    /// The live orchestrator; None means the task id.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub lead: Option<AgentId>,
    #[serde(default)]
    #[ts(as = "Option<Vec<BreakdownVersion>>", optional)]
    pub breakdowns: Vec<BreakdownVersion>,
    #[serde(default)]
    #[ts(as = "Option<Vec<FleetEvent>>", optional)]
    pub inbox: Vec<FleetEvent>,
    #[serde(default)]
    #[ts(as = "Option<Vec<GateRun>>", optional)]
    pub gate_runs: Vec<GateRun>,
    /// The PRs found on the task's branches, as the last successful check saw them.
    #[serde(default)]
    #[ts(as = "Option<Vec<TaskPr>>", optional)]
    pub prs: Vec<TaskPr>,
    /// When the task's PRs were seen merged, or when it was marked done by hand.
    #[serde(default)]
    #[ts(type = "number | null", optional)]
    pub done_at: Option<i64>,
    /// Marked done by the user: a task with no GitHub repo has no PR to merge.
    #[serde(default)]
    #[ts(as = "Option<bool>", optional)]
    pub done_by_hand: bool,
    /// The PR style resolved when the task started; None for records from before PR styles.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub pr_rules: Option<PrRules>,
}

/// What an orphan is: something the daemon made, or a tool left, that no task record owns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum OrphanKind {
    /// A folder directly under the tasks root, with the worktrees registered inside it.
    TaskFolder,
    /// A folder under the daemon's `agents/` directory.
    AgentFolder,
    /// A worktree under the tasks root outside any task folder, possibly with its folder gone.
    Worktree,
}

/// Something found with no task record. `id` is `"<kind>:<path>"`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct Orphan {
    pub id: String,
    pub kind: OrphanKind,
    /// The folder or worktree path.
    pub path: String,
    /// The registered worktrees inside a task folder.
    pub worktrees: Vec<String>,
    /// None until the background measurer has weighed the folder.
    #[serde(default)]
    #[ts(type = "number | null")]
    pub bytes: Option<u64>,
}

/// Everything the Tasks view shows: the records, what was found with no record, and the clone cache's size.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct TaskList {
    pub tasks: Vec<Task>,
    pub orphans: Vec<Orphan>,
    /// The size of the daemon's clone cache (`<state_dir>/repos`); None until the background measurer has walked it.
    #[serde(default)]
    #[ts(type = "number | null")]
    pub clones_bytes: Option<u64>,
}
