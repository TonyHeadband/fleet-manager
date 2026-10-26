use crate::{Actor, Agent, AgentKind, PlanItem};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// Where a remembered repo comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RepoSource {
    Path,
    Url,
}

/// A repo the daemon remembers, by name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct RepoInfo {
    pub name: String,
    pub source: RepoSource,
    /// The local path, or the clone URL.
    pub location: String,
    /// The persona that owns the repo; None for one not adopted by any persona yet.
    #[serde(default)]
    pub persona: Option<String>,
    /// Specialists the repo's config file names; empty when it has none.
    #[serde(default)]
    #[ts(as = "Option<Vec<String>>", optional)]
    pub roster: Vec<String>,
    /// The repo's `.fleet/workflow.yml` on the base a new task would use, as the clone last fetched it.
    pub workflow: WorkflowState,
}

/// What a repo's `.fleet/workflow.yml` holds on a base branch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
#[ts(export)]
pub enum WorkflowState {
    /// Valid; `steps` are the step names in run order.
    Found {
        base: String,
        steps: Vec<String>,
    },
    Missing {
        base: String,
    },
    Invalid {
        base: String,
        error: String,
    },
    /// The file could not be read at all, e.g. no `origin` remote or the base never fetched.
    Unreadable {
        base: String,
        error: String,
    },
}

/// The repos the daemon remembers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct RepoList {
    pub repos: Vec<RepoInfo>,
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
// `repos` replaced a single `repo`; an old client's `repo` would otherwise be dropped silently and the task
// start with no repo at all.
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct SpawnRequest {
    pub actor: Actor,
    /// Repos the task starts with; may be empty (the agent asks for access later).
    #[serde(default)]
    pub repos: Vec<String>,
    pub prompt: String,
    pub slug: Option<String>,
    /// Required. Empty is rejected: the daemon never picks a persona for the caller.
    #[serde(default)]
    #[ts(as = "Option<String>", optional)]
    pub persona: String,
    #[serde(default)]
    #[ts(as = "Option<AgentKind>", optional)]
    pub kind: AgentKind,
    /// The orchestrator type, for `kind` = `Orchestrator`.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub role: Option<String>,
    #[serde(default)]
    #[ts(as = "Option<Vec<String>>", optional)]
    pub roster: Vec<String>,
}

/// Body of `POST /api/agents`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct SpawnResponse {
    pub agent: Agent,
    /// What went wrong without stopping the spawn, one line each: a fetch that failed (the repo starts from the
    /// last fetched commit), repo settings Claude Code does not load from the task folder.
    pub warnings: Vec<String>,
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct AddRepoRequest {
    pub actor: Actor,
    /// A remembered name, a path or a URL.
    pub repo: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct GrantRepoRequest {
    pub actor: Actor,
    pub repo: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct RefuseRepoRequest {
    pub actor: Actor,
    #[serde(default)]
    pub reason: String,
}

/// Names one orphan from the last scan, by its `Orphan::id`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct OrphanRequest {
    pub id: String,
}

/// Body of `POST /api/tasks/{id}/archive`: the plan the user confirmed, as `archive-plan` returned it. The daemon
/// measures again and archives only if the same paths would go or stay, losing the same uncommitted files.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct ArchiveRequest {
    pub items: Vec<PlanItem>,
}

/// Body of `POST /api/orphans/archive`: the orphan, by its `Orphan::id`, and the plan the user confirmed for it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct ArchiveOrphanRequest {
    pub id: String,
    pub items: Vec<PlanItem>,
}
