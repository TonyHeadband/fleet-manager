use crate::{Actor, Agent, AgentKind, PlanItem};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// A repo of a product, by name. It has a local path, a clone URL, or both.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct RepoInfo {
    pub product: String,
    pub name: String,
    /// The main checkout on this machine; a task can start from one of its branches.
    pub path: Option<String>,
    /// The clone URL, credentials removed; a task started from it begins at `origin/HEAD` of a fresh fetch.
    pub url: Option<String>,
    /// The repo's `.fleet/workflow.yml` on the base a new task would use, as last fetched.
    pub workflow: WorkflowState,
    /// What a task of this repo runs with: the fleet env and the repo's `env:`, by name.
    #[serde(default)]
    pub env: Vec<EnvLine>,
}

/// Where an env value comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum EnvSource {
    /// Agent settings → Environment.
    Fleet,
    /// The repo's `.fleet/workflow.yml`.
    Repo,
}

/// One variable of a repo's effective env, as written (a `{task}/` value is not expanded).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct EnvLine {
    pub name: String,
    pub value: String,
    pub source: EnvSource,
    /// Whether the gate's build sees it: only the repo's own variables.
    pub gate: bool,
    /// A fleet value the repo sets again; the repo's line follows it.
    pub overridden: bool,
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

/// The repos of a product, or of every product.
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

/// One repo a task starts with, and where it starts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct RepoPick {
    /// A repo of the task's product by name, or a path or URL, which is added to the product first.
    pub repo: String,
    /// A checkout of the repo's local path to start from (its current branch). None starts from the URL's
    /// `origin/HEAD`, freshly fetched.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub local: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
// `repos` replaced a single `repo`; an old client's `repo` would otherwise be dropped silently and the task
// start with no repo at all.
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct SpawnRequest {
    pub actor: Actor,
    /// The product the task belongs to; required.
    pub product: String,
    /// A release of `product`; None puts the task in the product's backlog.
    #[serde(default)]
    #[ts(type = "number | null", optional)]
    pub release: Option<i64>,
    /// Repos the task starts with; may be empty (the agent asks for access later).
    #[serde(default)]
    #[ts(as = "Option<Vec<RepoPick>>", optional)]
    pub repos: Vec<RepoPick>,
    pub prompt: String,
    pub slug: Option<String>,
    /// The card's title; None or blank leaves the naming to the agent.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub title: Option<String>,
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
    /// A repo of the task's product by name, a path or a URL.
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
