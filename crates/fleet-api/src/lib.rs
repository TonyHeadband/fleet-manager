//! The fleet-manager client/daemon contract. Types only; no I/O.
mod event;
mod model;
mod project;
mod requests;
pub mod routes;
mod status;
mod ui;

pub use event::{Event, EventKind, Snapshot};
pub use model::{Actor, Agent, AgentId, RepoRequest, Workspace};
pub use project::{
    format_bytes, ArchivePlan, ArchiveReport, Footprint, FootprintItem, LogEntry, Orphan,
    OrphanKind, PlanItem, Project, ProjectList, ProjectState, StatusTimes,
};
pub use requests::{
    AddRepoRequest, AnswerRequest, ControlRequest, DiffReport, GrantRepoRequest, OrphanRequest,
    RefuseRepoRequest, RepoInfo, RepoList, RepoSource, SpawnRequest, SpawnResponse, WorkspaceDiff,
};
pub use status::{BlockedReason, Status, StopCause};
pub use ui::{
    default_ui_schema, eval, ActionDef, ColumnDef, Condition, FieldDef, FieldKind, Tone, UiSchema,
};

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        routes::list_agents,
        routes::spawn_agent,
        routes::stop_agent,
        routes::answer_agent,
        routes::pause_agent,
        routes::resume_agent,
        routes::terminal_stream,
        routes::event_stream,
        routes::ui_schema,
        routes::list_repos,
        routes::agent_diff,
        routes::add_repo,
        routes::grant_repo_request,
        routes::refuse_repo_request,
        routes::list_projects,
        routes::refresh_projects,
        routes::archive_plan,
        routes::archive_project,
        routes::orphan_plan,
        routes::archive_orphan,
        routes::end_orphan_session,
    ),
    components(schemas(
        Status,
        BlockedReason,
        AgentId,
        Actor,
        Agent,
        Event,
        EventKind,
        Snapshot,
        SpawnRequest,
        SpawnResponse,
        AnswerRequest,
        ControlRequest,
        RepoList,
        RepoInfo,
        RepoSource,
        RepoRequest,
        AddRepoRequest,
        GrantRepoRequest,
        RefuseRepoRequest,
        Workspace,
        Project,
        ProjectList,
        OrphanRequest,
        ProjectState,
        LogEntry,
        StatusTimes,
        FootprintItem,
        Footprint,
        ArchiveReport,
        ArchivePlan,
        PlanItem,
        Orphan,
        OrphanKind,
        DiffReport,
        WorkspaceDiff,
        UiSchema,
        ColumnDef,
        FieldDef,
        ActionDef,
        Condition,
        Tone,
        FieldKind
    ))
)]
struct ApiDoc;

pub fn openapi() -> utoipa::openapi::OpenApi {
    ApiDoc::openapi()
}
