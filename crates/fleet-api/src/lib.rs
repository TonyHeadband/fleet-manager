//! The fleet-board client/daemon contract. Types only; no I/O.
mod event;
mod model;
mod requests;
pub mod routes;
mod status;
mod ui;

pub use event::{Event, EventKind, Snapshot};
pub use model::{Actor, Agent, AgentId, Workspace};
pub use requests::{AnswerRequest, ControlRequest, DiffReport, RepoList, SpawnRequest, WorkspaceDiff};
pub use status::{BlockedReason, Status};
pub use ui::{
    default_ui_schema, eval, ActionDef, ColumnDef, Condition, FieldDef, FieldKind, Tone, UiSchema,
};

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        routes::list_agents,
        routes::spawn_agent,
        routes::kill_agent,
        routes::answer_agent,
        routes::pause_agent,
        routes::resume_agent,
        routes::terminal_stream,
        routes::event_stream,
        routes::ui_schema,
        routes::list_repos,
        routes::agent_diff,
    ),
    components(schemas(
        Status, BlockedReason, AgentId, Actor, Agent, Event, EventKind, Snapshot, SpawnRequest, AnswerRequest,
        ControlRequest, RepoList, Workspace, DiffReport, WorkspaceDiff, UiSchema, ColumnDef, FieldDef, ActionDef, Condition, Tone, FieldKind
    ))
)]
struct ApiDoc;

pub fn openapi() -> utoipa::openapi::OpenApi {
    ApiDoc::openapi()
}
