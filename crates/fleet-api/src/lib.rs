//! The fleet-manager client/daemon contract. Types only; no I/O.
mod event;
mod fleet;
mod model;
mod persona;
mod product;
mod progress;
mod requests;
pub mod routes;
mod status;
mod task;
mod ui;

pub use event::{Event, EventKind, Snapshot};
pub use fleet::{
    rollup, task_id, AgentKind, Approval, BreakdownVersion, DecideBreakdown, DefFile, DefInfo,
    DefKind, FleetEvent, FleetEventKind, FleetView, GateRun, GateStep, RepoFleet, Step, StepDraft,
    StepState, StopTaskResponse,
};
pub use model::{Actor, Agent, AgentId, RepoRequest, Workspace};
pub use persona::{
    ambient_vars, AgentConfig, AgentConfigUpdate, AgentSettings, GithubAuth, LoginState,
    NewPersona, Persona, PersonaList, PersonaUpdate, AMBIENT_GIT, AMBIENT_TOKENS,
};
pub use product::{
    NewProduct, NewRelease, Product, ProductList, ProductUpdate, Release, ReleaseUpdate,
    RepoUpsert, SetTaskRelease,
};
pub use progress::{parse_date, today};
pub use requests::{
    AddRepoRequest, AnswerRequest, ArchiveOrphanRequest, ArchiveRequest, ControlRequest,
    DiffReport, GrantRepoRequest, OrphanRequest, RefuseRepoRequest, RepoInfo, RepoList, RepoPick,
    SpawnRequest, SpawnResponse, WorkflowState, WorkspaceDiff,
};
pub use status::{BlockedReason, Status, StopCause};
pub use task::{
    format_bytes, ArchivePlan, ArchiveReport, Footprint, FootprintItem, LogEntry, Orphan,
    OrphanKind, PlanItem, StatusTimes, Task, TaskList, TaskState, PLAN_CHANGED,
};
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
        routes::list_products,
        routes::list_repos,
        routes::create_product,
        routes::update_product,
        routes::delete_product,
        routes::upsert_repo,
        routes::delete_repo,
        routes::set_repo_fleet,
        routes::create_release,
        routes::update_release,
        routes::delete_release,
        routes::set_task_release,
        routes::agent_diff,
        routes::add_repo,
        routes::grant_repo_request,
        routes::refuse_repo_request,
        routes::list_tasks,
        routes::refresh_tasks,
        routes::archive_plan,
        routes::archive_task,
        routes::orphan_plan,
        routes::archive_orphan,
        routes::end_orphan_session,
        routes::list_personas,
        routes::create_persona,
        routes::update_persona,
        routes::delete_persona,
        routes::check_persona,
        routes::get_agent_config,
        routes::put_agent_config,
        routes::list_definitions,
        routes::get_definition,
        routes::put_definition,
        routes::reset_definition,
        routes::fleet_view,
        routes::decide_breakdown,
        routes::stop_task,
        routes::restart_orchestrator,
    ),
    components(schemas(
        Status,
        BlockedReason,
        StopCause,
        AgentKind,
        StepState,
        Step,
        StepDraft,
        Approval,
        BreakdownVersion,
        FleetEventKind,
        FleetEvent,
        GateStep,
        GateRun,
        FleetView,
        RepoFleet,
        DefKind,
        DefInfo,
        DefFile,
        DecideBreakdown,
        StopTaskResponse,
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
        WorkflowState,
        RepoPick,
        Product,
        ProductList,
        Release,
        NewProduct,
        ProductUpdate,
        NewRelease,
        ReleaseUpdate,
        RepoUpsert,
        SetTaskRelease,
        RepoRequest,
        AddRepoRequest,
        GrantRepoRequest,
        RefuseRepoRequest,
        Workspace,
        GithubAuth,
        LoginState,
        Persona,
        PersonaList,
        NewPersona,
        PersonaUpdate,
        AgentSettings,
        AgentConfig,
        AgentConfigUpdate,
        Task,
        TaskList,
        OrphanRequest,
        ArchiveRequest,
        ArchiveOrphanRequest,
        TaskState,
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
