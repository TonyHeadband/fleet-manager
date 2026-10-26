//! Contract-only route descriptions. The daemon implements these; clients and docs read them.
use crate::{
    AddRepoRequest, Agent, AgentConfig, AgentConfigUpdate, AnswerRequest, ArchiveOrphanRequest,
    ArchivePlan, ArchiveReport, ArchiveRequest, ControlRequest, DecideBreakdown, DefFile, DefInfo,
    DefKind, DiffReport, Event, FleetView, GrantRepoRequest, NewPersona, OrphanRequest, Persona,
    PersonaList, PersonaUpdate, ProjectList, RefuseRepoRequest, RepoFleet, RepoInfo, RepoList,
    Snapshot, SpawnRequest, SpawnResponse, StopProjectResponse, UiSchema, Workspace,
};

#[utoipa::path(get, path = "/api/agents", responses((status = 200, body = Snapshot)))]
pub fn list_agents() {}

#[utoipa::path(post, path = "/api/agents", request_body = SpawnRequest, responses((status = 200, body = SpawnResponse)))]
pub fn spawn_agent() {}

#[utoipa::path(post, path = "/api/agents/{id}/stop", params(("id" = String, Path)), responses((status = 204)))]
pub fn stop_agent() {}

#[utoipa::path(post, path = "/api/agents/{id}/answer", params(("id" = String, Path)), request_body = AnswerRequest, responses((status = 204)))]
pub fn answer_agent() {}

#[utoipa::path(post, path = "/api/agents/{id}/pause", params(("id" = String, Path)), request_body = ControlRequest, responses((status = 204)))]
pub fn pause_agent() {}

#[utoipa::path(post, path = "/api/agents/{id}/resume", params(("id" = String, Path)), request_body = ControlRequest, responses((status = 204)))]
pub fn resume_agent() {}

#[utoipa::path(get, path = "/api/agents/{id}/terminal", params(("id" = String, Path)), responses((status = 200, content_type = "text/event-stream", body = String)))]
pub fn terminal_stream() {}

#[utoipa::path(get, path = "/api/events", responses((status = 200, content_type = "text/event-stream", body = Event)))]
pub fn event_stream() {}

#[utoipa::path(get, path = "/api/ui-schema", responses((status = 200, body = UiSchema)))]
pub fn ui_schema() {}

#[utoipa::path(get, path = "/api/repos", params(("persona" = Option<String>, Query, description = "Only this persona's repos")), responses((status = 200, body = RepoList)))]
pub fn list_repos() {}

/// Sets the roster a repo offers orchestrated projects. Every roster name must be a specialist that loads (400 names
/// the first that does not). 404 for a repo the persona does not remember.
#[utoipa::path(put, path = "/api/repos/{persona}/{name}/fleet", params(("persona" = String, Path), ("name" = String, Path)), request_body = RepoFleet, responses((status = 200, body = RepoInfo), (status = 400), (status = 404)))]
pub fn set_repo_fleet() {}

#[utoipa::path(get, path = "/api/agents/{id}/diff", params(("id" = String, Path)), responses((status = 200, body = DiffReport)))]
pub fn agent_diff() {}

#[utoipa::path(post, path = "/api/agents/{id}/repos", params(("id" = String, Path)), request_body = AddRepoRequest, responses((status = 200, body = Workspace)))]
pub fn add_repo() {}

#[utoipa::path(post, path = "/api/agents/{id}/repo-request/grant", params(("id" = String, Path)), request_body = GrantRepoRequest, responses((status = 200, body = Workspace)))]
pub fn grant_repo_request() {}

#[utoipa::path(post, path = "/api/agents/{id}/repo-request/refuse", params(("id" = String, Path)), request_body = RefuseRepoRequest, responses((status = 204)))]
pub fn refuse_repo_request() {}

#[utoipa::path(get, path = "/api/projects", responses((status = 200, body = ProjectList)))]
pub fn list_projects() {}

#[utoipa::path(post, path = "/api/projects/refresh", responses((status = 202)))]
pub fn refresh_projects() {}

#[utoipa::path(get, path = "/api/projects/{id}/archive-plan", params(("id" = String, Path)), responses((status = 200, body = ArchivePlan), (status = 404), (status = 409)))]
pub fn archive_plan() {}

/// Archives only against the plan the user confirmed; 409 with `PLAN_CHANGED` when the folder changed since.
#[utoipa::path(post, path = "/api/projects/{id}/archive", params(("id" = String, Path)), request_body = ArchiveRequest, responses((status = 200, body = ArchiveReport), (status = 404), (status = 409)))]
pub fn archive_project() {}

#[utoipa::path(post, path = "/api/orphans/plan", request_body = OrphanRequest, responses((status = 200, body = ArchivePlan), (status = 404)))]
pub fn orphan_plan() {}

/// Archives only against the plan the user confirmed; 409 with `PLAN_CHANGED` when the orphan changed since.
#[utoipa::path(post, path = "/api/orphans/archive", request_body = ArchiveOrphanRequest, responses((status = 200, body = ArchiveReport), (status = 404), (status = 409)))]
pub fn archive_orphan() {}

#[utoipa::path(post, path = "/api/orphans/end-session", request_body = OrphanRequest, responses((status = 204), (status = 404)))]
pub fn end_orphan_session() {}

#[utoipa::path(get, path = "/api/personas", responses((status = 200, body = PersonaList)))]
pub fn list_personas() {}

/// The persona starts as a draft: `ready` turns true on the first check that finds both logins.
#[utoipa::path(post, path = "/api/personas", request_body = NewPersona, responses((status = 200, body = Persona), (status = 400)))]
pub fn create_persona() {}

/// The id and the host never change.
#[utoipa::path(patch, path = "/api/personas/{id}", params(("id" = String, Path)), request_body = PersonaUpdate, responses((status = 200, body = Persona), (status = 400), (status = 404)))]
pub fn update_persona() {}

/// 409 while the persona still has running or stopped projects.
#[utoipa::path(delete, path = "/api/personas/{id}", params(("id" = String, Path)), responses((status = 204), (status = 404), (status = 409)))]
pub fn delete_persona() {}

/// Asks the `claude` and `gh` CLIs whether the persona is logged in; the first success makes it ready.
#[utoipa::path(post, path = "/api/personas/{id}/check", params(("id" = String, Path)), responses((status = 200, body = Persona), (status = 404)))]
pub fn check_persona() {}

/// The instructions and settings every agent gets, whatever its persona.
#[utoipa::path(get, path = "/api/agent-config", responses((status = 200, body = AgentConfig)))]
pub fn get_agent_config() {}

/// 400, with the reason in the body, when a rule or an env name is not allowed.
#[utoipa::path(put, path = "/api/agent-config", request_body = AgentConfigUpdate, responses((status = 200, body = AgentConfig), (status = 400)))]
pub fn put_agent_config() {}

/// Built-in and user orchestrator types, specialists and skills; one that does not load carries its `error`.
#[utoipa::path(get, path = "/api/definitions", responses((status = 200, body = Vec<DefInfo>)))]
pub fn list_definitions() {}

/// The user's copy when there is one, else the built-in (`builtin: true`, shown read-only). For a skill, its
/// `SKILL.md`.
#[utoipa::path(get, path = "/api/definitions/{kind}/{name}", params(("kind" = DefKind, Path), ("name" = String, Path)), responses((status = 200, body = DefFile), (status = 404)))]
pub fn get_definition() {}

/// The body is the file's text (Markdown with frontmatter). It is validated first; 400 names the file and the
/// problem. Saving under a built-in's name overrides it.
#[utoipa::path(put, path = "/api/definitions/{kind}/{name}", params(("kind" = DefKind, Path), ("name" = String, Path)), request_body(content = String, content_type = "text/markdown"), responses((status = 200, body = DefInfo), (status = 400)))]
pub fn put_definition() {}

/// Deletes the user's copy, so the built-in applies again. 404 when there is no user copy, built-in or not.
#[utoipa::path(delete, path = "/api/definitions/{kind}/{name}", params(("kind" = DefKind, Path), ("name" = String, Path)), responses((status = 204), (status = 404)))]
pub fn reset_definition() {}

/// Every breakdown version and gate run of an orchestrated project; a solo project has none.
#[utoipa::path(get, path = "/api/projects/{id}/fleet", params(("id" = String, Path)), responses((status = 200, body = FleetView), (status = 404)))]
pub fn fleet_view() {}

/// Approves the proposed breakdown or asks for changes; 409 when none is waiting for a decision.
#[utoipa::path(post, path = "/api/projects/{id}/breakdown/decide", params(("id" = String, Path)), request_body = DecideBreakdown, responses((status = 204), (status = 404), (status = 409)))]
pub fn decide_breakdown() {}

/// Stops every agent of the project still running, its fleet first and its orchestrator last. Agents already stopped
/// are not counted, so a second call answers 0.
#[utoipa::path(post, path = "/api/projects/{id}/stop", params(("id" = String, Path)), responses((status = 200, body = StopProjectResponse), (status = 404)))]
pub fn stop_project() {}

/// Starts a fresh orchestrator in the project's orchestrator folder, with the stored breakdown, step reports and
/// pending fleet events in its prompt. 409 while the orchestrator still runs, or for a solo project.
#[utoipa::path(post, path = "/api/projects/{id}/restart", params(("id" = String, Path)), responses((status = 200, body = Agent), (status = 404), (status = 409)))]
pub fn restart_orchestrator() {}
