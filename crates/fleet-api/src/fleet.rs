use crate::{Agent, AgentId, Status};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// What an agent is in a fleet; `Solo` is every agent that predates fleets.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, TS,
)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AgentKind {
    #[default]
    Solo,
    Orchestrator,
    Specialist,
}

/// The id of the task record this agent belongs to: its own id unless a fleet owns it.
pub fn task_id(a: &Agent) -> &AgentId {
    a.task.as_ref().unwrap_or(&a.id)
}

/// The most urgent of a task's agents' statuses, for its card. `Talking` counts as `Working`; an empty set
/// (every agent ended) is `Stopped`.
pub fn rollup(statuses: impl IntoIterator<Item = Status>) -> Status {
    fn urgency(s: Status) -> u8 {
        match s {
            Status::NeedsYou => 4,
            Status::Working | Status::Talking => 3,
            Status::Queued => 2,
            Status::Review => 1,
            Status::Stopped => 0,
        }
    }
    let top = statuses
        .into_iter()
        .max_by_key(|s| urgency(*s))
        .unwrap_or(Status::Stopped);
    if top == Status::Talking {
        Status::Working
    } else {
        top
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum StepState {
    Pending,
    Running,
    Done,
    Failed,
}

/// One step of an approved breakdown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct Step {
    pub n: u32,
    pub title: String,
    pub specialist: String,
    pub brief: String,
    #[serde(default)]
    pub depends_on: Vec<u32>,
    /// The step's PR carries a release-notes line.
    #[serde(default)]
    pub release_note: bool,
    pub state: StepState,
    /// The specialist agent running or that ran this step.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub agent: Option<AgentId>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub branch: Option<String>,
    /// The specialist's finishing report.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub report: Option<String>,
}

/// A step as the orchestrator proposes it, before the daemon assigns state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct StepDraft {
    /// `Some` marks a step carried over from the previous version, keeping its number and progress.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub n: Option<u32>,
    pub title: String,
    pub specialist: String,
    pub brief: String,
    #[serde(default)]
    pub depends_on: Vec<u32>,
    /// The step's PR carries a release-notes line.
    #[serde(default)]
    pub release_note: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Approval {
    Proposed,
    Approved,
    ChangesRequested,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct BreakdownVersion {
    pub version: u32,
    pub steps: Vec<Step>,
    pub approval: Approval,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub comment: Option<String>,
    #[ts(type = "number")]
    pub proposed_at: i64,
    #[serde(default)]
    #[ts(type = "number | null", optional)]
    pub decided_at: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FleetEventKind {
    Finished,
    Failed,
    NeedsYou,
    IdleWithoutFinish,
    /// The owner decided a breakdown version no held `propose_breakdown` call was left to hear. Not a step's:
    /// `step` is 0 and `agent` is the task.
    BreakdownDecided,
}

/// Something a fleet agent did that the orchestrator should hear about.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct FleetEvent {
    #[ts(type = "number")]
    pub at: i64,
    pub step: u32,
    pub agent: AgentId,
    pub kind: FleetEventKind,
    pub detail: String,
}

/// One workflow step the gate ran.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct GateStep {
    pub name: String,
    pub command: String,
    /// The changed files that picked it; empty for a step without `paths`, or one that ran only for `unmatched`.
    pub because: Vec<String>,
    /// `None` means the step timed out.
    #[ts(type = "number | null")]
    pub exit_code: Option<i32>,
    #[ts(type = "number")]
    pub duration_secs: u64,
    pub output_tail: String,
    /// Only on the `pr` step: it refused for rules the orchestrator fixes itself, so it counts toward
    /// `max_pr_attempts`.
    #[serde(default)]
    pub counted: bool,
}

/// One run of the repo's workflow (`.fleet/workflow.yml` on the base branch) before opening a PR.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct GateRun {
    #[ts(type = "number")]
    pub at: i64,
    /// The branch the workflow was read from, and the PR's base.
    #[serde(default)]
    pub base: String,
    /// In run order; it stops at the first step that fails.
    #[serde(default)]
    pub steps: Vec<GateStep>,
    /// Changed files no step covers; any made every step run.
    #[serde(default)]
    pub unmatched: Vec<String>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub pr_url: Option<String>,
}

impl GateRun {
    /// A run with no step never passes: nothing was checked.
    pub fn passed(&self) -> bool {
        !self.steps.is_empty() && self.steps.iter().all(|s| s.exit_code == Some(0))
    }
}

/// What the fleet view shows for one task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct FleetView {
    pub task: AgentId,
    pub breakdowns: Vec<BreakdownVersion>,
    pub gate_runs: Vec<GateRun>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DefKind {
    Orchestrator,
    Specialist,
    Skill,
    PrStyle,
}

/// One orchestrator, specialist, skill or PR style definition in the list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct DefInfo {
    pub kind: DefKind,
    pub name: String,
    pub summary: String,
    pub builtin: bool,
    /// True when a user file shadows the built-in of the same name.
    pub overridden: bool,
    /// The model its agents run on; none for a skill or when the file names none.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub model: Option<String>,
    /// Why the definition cannot be used, when it cannot.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct DefFile {
    pub kind: DefKind,
    pub name: String,
    pub text: String,
    pub builtin: bool,
}

/// What stopping a whole task did: how many of its agents were still running and are stopped now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct StopTaskResponse {
    pub stopped: u32,
}

/// Body of the owner's decision on the proposed breakdown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct DecideBreakdown {
    pub approve: bool,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub comment: Option<String>,
    /// The version the owner reviewed. When set, the decision is refused unless that version is the one waiting,
    /// so a proposal that replaced it meanwhile is never approved unseen.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub version: Option<u32>,
}
