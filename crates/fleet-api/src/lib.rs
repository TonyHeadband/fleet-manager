//! The fleet-manager client/daemon contract. Types only; no I/O.
mod chat;
mod event;
mod fleet;
mod form;
mod internal;
mod mode;
mod model;
mod persona;
mod pr;
mod product;
mod progress;
mod requests;
pub mod routes;
mod status;
mod task;
mod tool;
mod ui;

pub use chat::{CallStatus, ChatBody, ChatItem, ChatPage, MarkerKind, PromptFrom};
pub use event::{Event, EventKind, Snapshot};
pub use fleet::{
    rollup, task_id, AgentKind, Approval, BreakdownVersion, DecideBreakdown, DefFile, DefInfo,
    DefKind, FleetEvent, FleetEventKind, FleetView, GateRun, GateStep, Step, StepDraft, StepState,
    StopTaskResponse,
};
pub use form::{
    breakdown_decision, check, check_submit, instructions, release_create, release_edit,
    repo_request, ChoiceSource, DefaultValue, FieldError, FixedValue, FormDef, FormItem, InputDef,
    InputKind, Rule, SubmitDef,
};
pub use internal::{
    InternalBranches, InternalCommit, InternalFile, InternalFiles, InternalHistory, InternalRemote,
};
pub use mode::{Effort, EffortError, ModeError, PermissionMode};
pub use model::{Actor, Agent, AgentId, RepoRequest, Workspace};
pub use persona::{
    ambient_vars, AgentConfig, AgentConfigUpdate, AgentSettings, GithubAuth, LoginState,
    NewPersona, Persona, PersonaList, PersonaUpdate, AMBIENT_GIT, AMBIENT_TOKENS,
};
pub use pr::{PrRules, Versioning};
pub use product::{
    EffectiveSettings, EffectiveValue, HiddenSpecialistsPut, NewProduct, NewRelease, OverridesPut,
    Product, ProductList, ProductOverride, ProductUpdate, Release, ReleaseUpdate, RepoUpsert,
    SetTaskRelease, SettingSource,
};
pub use progress::{
    parse_date, release_progress, task_outcome, task_progress, today, ReleaseProgress, TaskProgress,
};
pub use requests::{
    AddRepoRequest, AnswerRequest, ArchiveOrphanRequest, ArchiveRequest, CheckoutInfo,
    CheckoutList, ControlRequest, DiffReport, EnvLine, EnvSource, GrantRepoRequest, OrphanRequest,
    RefuseRepoRequest, RepoInfo, RepoList, RepoPick, SpawnRequest, SpawnResponse, WorkflowState,
    WorkspaceDiff,
};
pub use status::{BlockedReason, Status, StopCause};
pub use task::{
    format_bytes, ArchivePlan, ArchiveReport, Footprint, FootprintItem, LogEntry, Orphan,
    OrphanKind, PlanItem, PrState, StatusTimes, Task, TaskList, TaskOutcome, TaskPr, TaskState,
    PLAN_CHANGED,
};
pub use tool::{Os, ToolError, ToolFix, ToolId, ToolList, ToolStatus, VersionInfo};
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
        routes::repo_checkouts,
        routes::put_hidden_specialists,
        routes::put_product_overrides,
        routes::effective_settings,
        routes::internal_branches,
        routes::internal_files,
        routes::internal_file,
        routes::internal_history,
        routes::create_release,
        routes::update_release,
        routes::delete_release,
        routes::set_task_release,
        routes::mark_done,
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
        routes::list_tools,
        routes::recheck_tools,
        routes::daemon_version,
    ),
    components(schemas(
        Status,
        BlockedReason,
        StopCause,
        AgentKind,
        PermissionMode,
        Effort,
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
        InternalBranches,
        InternalCommit,
        InternalFile,
        InternalFiles,
        InternalHistory,
        InternalRemote,
        DefKind,
        PrRules,
        Versioning,
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
        CheckoutInfo,
        CheckoutList,
        WorkflowState,
        EnvLine,
        EnvSource,
        RepoPick,
        Product,
        ProductList,
        Release,
        NewProduct,
        ProductUpdate,
        ProductOverride,
        OverridesPut,
        HiddenSpecialistsPut,
        SettingSource,
        EffectiveValue,
        EffectiveSettings,
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
        TaskPr,
        PrState,
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
        FieldKind,
        ToolId,
        ToolFix,
        ToolError,
        ToolStatus,
        ToolList,
        VersionInfo,
        Os,
        ChatItem,
        ChatBody,
        PromptFrom,
        CallStatus,
        MarkerKind,
        ChatPage
    ))
)]
struct ApiDoc;

pub fn openapi() -> utoipa::openapi::OpenApi {
    ApiDoc::openapi()
}
