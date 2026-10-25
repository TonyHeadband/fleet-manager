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
    ArchiveReport, Footprint, FootprintItem, LogEntry, Orphan, OrphanKind, Project, ProjectState,
    StatusTimes,
};
pub use requests::{
    AddRepoRequest, AnswerRequest, ControlRequest, DiffReport, GrantRepoRequest, RefuseRepoRequest,
    RepoInfo, RepoList, RepoSource, SpawnRequest, SpawnResponse, WorkspaceDiff,
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
        ProjectState,
        LogEntry,
        StatusTimes,
        FootprintItem,
        Footprint,
        ArchiveReport,
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
