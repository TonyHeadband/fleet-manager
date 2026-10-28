//! Contract-only route descriptions. The daemon implements these; clients and docs read them.
use crate::{
    AddRepoRequest, Agent, AgentConfig, AgentConfigUpdate, AnswerRequest, ArchiveOrphanRequest,
    ArchivePlan, ArchiveReport, ArchiveRequest, ControlRequest, DecideBreakdown, DefFile, DefInfo,
    DefKind, DiffReport, EffectiveSettings, Event, FleetView, GrantRepoRequest, InternalBranches,
    InternalFile, InternalFiles, InternalHistory, NewPersona, NewProduct, NewRelease,
    OrphanRequest, OverridesPut, Persona, PersonaList, PersonaUpdate, Product, ProductList,
    ProductUpdate, RefuseRepoRequest, Release, ReleaseUpdate, RepoFleet, RepoInfo, RepoList,
    RepoUpsert, SetTaskRelease, Snapshot, SpawnRequest, SpawnResponse, StopTaskResponse, Task,
    TaskList, UiSchema, Workspace,
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

/// Every product with its repos and releases, oldest first.
#[utoipa::path(get, path = "/api/products", responses((status = 200, body = ProductList)))]
pub fn list_products() {}

/// The id is the name's slug, suffixed `-2`, `-3`... when taken. 400 for an empty name or an unknown default
/// persona; 409 when the name is taken.
#[utoipa::path(post, path = "/api/products", request_body = NewProduct, responses((status = 200, body = Product), (status = 400), (status = 409)))]
pub fn create_product() {}

/// `default_persona: null` clears it; leaving it out keeps it. 404 for an unknown product; 409 when the name is taken.
#[utoipa::path(patch, path = "/api/products/{id}", params(("id" = String, Path)), request_body = ProductUpdate, responses((status = 200, body = Product), (status = 400), (status = 404), (status = 409)))]
pub fn update_product() {}

/// 409 while the product still has task records, archived ones included. Deletes its repos and releases with it.
#[utoipa::path(delete, path = "/api/products/{id}", params(("id" = String, Path)), responses((status = 204), (status = 404), (status = 409)))]
pub fn delete_product() {}

/// Adds a repo, or adds a path to a URL-only repo (or a URL to a path-only one). 400 for neither, a path that is not
/// a git toplevel, or a URL with credentials, over SSH or plain HTTP. 409 when the repo already has a different URL.
#[utoipa::path(put, path = "/api/products/{id}/repos", params(("id" = String, Path)), request_body = RepoUpsert, responses((status = 200, body = RepoInfo), (status = 400), (status = 404), (status = 409)))]
pub fn upsert_repo() {}

/// 409 while a running task of the product uses the repo.
#[utoipa::path(delete, path = "/api/products/{id}/repos/{name}", params(("id" = String, Path), ("name" = String, Path)), responses((status = 204), (status = 404), (status = 409)))]
pub fn delete_repo() {}

/// Dates are `YYYY-MM-DD`. 400 for a bad date or a target before the start; 409 for a version the product has.
#[utoipa::path(post, path = "/api/products/{id}/releases", params(("id" = String, Path)), request_body = NewRelease, responses((status = 200, body = Release), (status = 400), (status = 404), (status = 409)))]
pub fn create_release() {}

/// `released: true` stamps `released_at` with now, `false` clears it.
#[utoipa::path(patch, path = "/api/releases/{id}", params(("id" = i64, Path)), request_body = ReleaseUpdate, responses((status = 200, body = Release), (status = 400), (status = 404), (status = 409)))]
pub fn update_release() {}

/// The release's tasks go back to the product's backlog.
#[utoipa::path(delete, path = "/api/releases/{id}", params(("id" = i64, Path)), responses((status = 204), (status = 404)))]
pub fn delete_release() {}

/// `release: null` moves the task to the backlog. 400 for a release of another product; 409 once the task is
/// finished (its PRs merged, or marked done), as it stays in the release it finished in.
#[utoipa::path(put, path = "/api/tasks/{id}/release", params(("id" = String, Path)), request_body = SetTaskRelease, responses((status = 200, body = Task), (status = 400), (status = 404), (status = 409)))]
pub fn set_task_release() {}

/// Marks a task done by hand. Only for a task none of whose repos has a URL (409 otherwise): one with a GitHub repo
/// is done when its PR merges.
#[utoipa::path(post, path = "/api/tasks/{id}/done", params(("id" = String, Path)), responses((status = 200, body = Task), (status = 404), (status = 409)))]
pub fn mark_done() {}

/// The repos a task may be started with: one product's, or every product's. Repos an agent made inside a task
/// folder are left out.
#[utoipa::path(get, path = "/api/repos", params(("product" = Option<String>, Query, description = "Only this product's repos")), responses((status = 200, body = RepoList)))]
pub fn list_repos() {}

/// Sets the roster a repo offers orchestrated tasks. Every roster name must be a specialist that loads (400 names
/// the first that does not). 404 for a repo the product does not have.
#[utoipa::path(put, path = "/api/products/{id}/repos/{name}/fleet", params(("id" = String, Path), ("name" = String, Path)), request_body = RepoFleet, responses((status = 200, body = RepoInfo), (status = 400), (status = 404)))]
pub fn set_repo_fleet() {}

/// Replaces a product's overrides. One bad entry (unknown section or key, bad value, a definition that does not
/// exist) refuses the whole list with a 400 naming it.
#[utoipa::path(put, path = "/api/products/{id}/overrides", params(("id" = String, Path)), request_body = OverridesPut, responses((status = 200, body = Product), (status = 400), (status = 404)))]
pub fn put_product_overrides() {}

/// Every setting an override can change, with the value that applies and the layer it came from. Without
/// `product`, only the global and definition layers.
#[utoipa::path(get, path = "/api/settings/effective", params(("product" = Option<String>, Query, description = "Apply this product's overrides")), responses((status = 200, body = EffectiveSettings), (status = 404)))]
pub fn effective_settings() {}

#[utoipa::path(get, path = "/api/agents/{id}/diff", params(("id" = String, Path)), responses((status = 200, body = DiffReport)))]
pub fn agent_diff() {}

#[utoipa::path(post, path = "/api/agents/{id}/repos", params(("id" = String, Path)), request_body = AddRepoRequest, responses((status = 200, body = Workspace)))]
pub fn add_repo() {}

#[utoipa::path(post, path = "/api/agents/{id}/repo-request/grant", params(("id" = String, Path)), request_body = GrantRepoRequest, responses((status = 200, body = Workspace)))]
pub fn grant_repo_request() {}

#[utoipa::path(post, path = "/api/agents/{id}/repo-request/refuse", params(("id" = String, Path)), request_body = RefuseRepoRequest, responses((status = 204)))]
pub fn refuse_repo_request() {}

#[utoipa::path(get, path = "/api/tasks", responses((status = 200, body = TaskList)))]
pub fn list_tasks() {}

/// Rescans for orphans, re-measures, and checks the open tasks' PRs, in the background.
#[utoipa::path(post, path = "/api/tasks/refresh", responses((status = 202)))]
pub fn refresh_tasks() {}

#[utoipa::path(get, path = "/api/tasks/{id}/archive-plan", params(("id" = String, Path)), responses((status = 200, body = ArchivePlan), (status = 404), (status = 409)))]
pub fn archive_plan() {}

/// Archives only against the plan the user confirmed; 409 with `PLAN_CHANGED` when the folder changed since.
#[utoipa::path(post, path = "/api/tasks/{id}/archive", params(("id" = String, Path)), request_body = ArchiveRequest, responses((status = 200, body = ArchiveReport), (status = 404), (status = 409)))]
pub fn archive_task() {}

#[utoipa::path(post, path = "/api/orphans/plan", request_body = OrphanRequest, responses((status = 200, body = ArchivePlan), (status = 404)))]
pub fn orphan_plan() {}

/// Archives only against the plan the user confirmed; 409 with `PLAN_CHANGED` when the orphan changed since.
#[utoipa::path(post, path = "/api/orphans/archive", request_body = ArchiveOrphanRequest, responses((status = 200, body = ArchiveReport), (status = 404), (status = 409)))]
pub fn archive_orphan() {}

#[utoipa::path(get, path = "/api/personas", responses((status = 200, body = PersonaList)))]
pub fn list_personas() {}

/// The persona starts as a draft: `ready` turns true on the first check that finds both logins.
#[utoipa::path(post, path = "/api/personas", request_body = NewPersona, responses((status = 200, body = Persona), (status = 400)))]
pub fn create_persona() {}

/// The id and the host never change.
#[utoipa::path(patch, path = "/api/personas/{id}", params(("id" = String, Path)), request_body = PersonaUpdate, responses((status = 200, body = Persona), (status = 400), (status = 404)))]
pub fn update_persona() {}

/// 409 while the persona still has running or stopped tasks.
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

/// Every breakdown version and gate run of an orchestrated task; a solo task has none.
#[utoipa::path(get, path = "/api/tasks/{id}/fleet", params(("id" = String, Path)), responses((status = 200, body = FleetView), (status = 404)))]
pub fn fleet_view() {}

/// Approves the proposed breakdown or asks for changes; 409 when none is waiting for a decision.
#[utoipa::path(post, path = "/api/tasks/{id}/breakdown/decide", params(("id" = String, Path)), request_body = DecideBreakdown, responses((status = 204), (status = 404), (status = 409)))]
pub fn decide_breakdown() {}

/// Stops every agent of the task still running, its fleet first and its orchestrator last. Agents already stopped
/// are not counted, so a second call answers 0.
#[utoipa::path(post, path = "/api/tasks/{id}/stop", params(("id" = String, Path)), responses((status = 200, body = StopTaskResponse), (status = 404)))]
pub fn stop_task() {}

/// Starts a fresh orchestrator in the task's orchestrator folder, with the stored breakdown, step reports and
/// pending fleet events in its prompt. 409 while the orchestrator still runs, or for a solo task.
#[utoipa::path(post, path = "/api/tasks/{id}/restart", params(("id" = String, Path)), responses((status = 200, body = Agent), (status = 404), (status = 409)))]
pub fn restart_orchestrator() {}

/// The branches of the product's internal repo; empty when the product has none yet.
#[utoipa::path(get, path = "/api/products/{id}/internal/branches", params(("id" = String, Path)), responses((status = 200, body = InternalBranches), (status = 404)))]
pub fn internal_branches() {}

/// The files on one branch (default `main`).
#[utoipa::path(get, path = "/api/products/{id}/internal/files", params(("id" = String, Path), ("branch" = Option<String>, Query, description = "Default main")), responses((status = 200, body = InternalFiles), (status = 404)))]
pub fn internal_files() {}

/// One text file up to 1 MiB; a path with `..`, an absolute path or a symlink, a larger file or a binary one is a 400.
#[utoipa::path(get, path = "/api/products/{id}/internal/file", params(("id" = String, Path), ("branch" = Option<String>, Query, description = "Default main"), ("path" = String, Query)), responses((status = 200, body = InternalFile), (status = 400), (status = 404)))]
pub fn internal_file() {}

/// Commits of a branch, or of one path on it, newest first.
#[utoipa::path(get, path = "/api/products/{id}/internal/history", params(("id" = String, Path), ("branch" = Option<String>, Query, description = "Default main"), ("path" = Option<String>, Query), ("limit" = Option<usize>, Query, description = "Default 50, at most 200")), responses((status = 200, body = InternalHistory), (status = 400), (status = 404)))]
pub fn internal_history() {}
