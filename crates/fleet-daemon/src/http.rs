//! The daemon's HTTP surface: JSON commands, SSE event and terminal streams, and the hook ingress.
use crate::agent_config;
use crate::agents::{AgentService, ServiceError, SpawnError};
use crate::fleet::defs::{Defs, FleetError};
use crate::git::GitError;
use crate::ingest::{authenticate, ingest, IngestError, IngestOutcome};
use crate::personas::PersonaError;
use crate::repos::{redact_url, RepoError};
use crate::store::Store;
use crate::tmux::Tmux;
use axum::extract::{Path, Query, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event as SseEvent, KeepAlive, KeepAliveStream, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use fleet_api::{
    default_ui_schema, openapi, AddRepoRequest, AgentConfig, AgentConfigUpdate, AgentId,
    AnswerRequest, ArchiveOrphanRequest, ArchivePlan, ArchiveReport, ArchiveRequest,
    ControlRequest, DecideBreakdown, DefFile, DefInfo, DefKind, EventKind, FleetView,
    GrantRepoRequest, NewPersona, OrphanKind, OrphanRequest, Persona, PersonaList, PersonaUpdate,
    ProjectList, RefuseRepoRequest, RepoInfo, RepoList, Snapshot, SpawnRequest, SpawnResponse,
    Status, Workspace,
};
use serde_json::{json, Value};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::ReceiverStream;
use tracing::Instrument;

const TERMINAL_POLL: Duration = Duration::from_millis(500);

#[derive(Clone)]
pub struct AppState {
    svc: Arc<AgentService>,
    store: Arc<Store>,
    tmux: Tmux,
    port: u16,
    terminals: Arc<AtomicUsize>,
    keep_alive: Duration,
}

impl AppState {
    /// `port` is the port the listener is bound to; the Host/Origin guard compares against it.
    pub fn new(svc: Arc<AgentService>, store: Arc<Store>, tmux: Tmux, port: u16) -> AppState {
        AppState {
            svc,
            store,
            tmux,
            port,
            terminals: Arc::new(AtomicUsize::new(0)),
            keep_alive: KEEP_ALIVE,
        }
    }

    /// How often a quiet event stream pings; tests shorten it.
    #[doc(hidden)]
    pub fn with_keep_alive(mut self, every: Duration) -> AppState {
        self.keep_alive = every;
        self
    }

    /// Open terminal-preview streams; tests use it to prove a gone client does not leave a poller running.
    pub fn active_terminal_streams(&self) -> usize {
        self.terminals.load(Ordering::SeqCst)
    }
}

/// v1 has no authentication, so the only safe listener is a loopback one.
pub async fn bind_loopback(addr: SocketAddr) -> std::io::Result<TcpListener> {
    if !addr.ip().is_loopback() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{addr} is not a loopback address; refusing to listen without authentication"),
        ));
    }
    TcpListener::bind(addr).await
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/agents", get(list_agents).post(spawn_agent))
        // Kill's old route: refused outright, so an old client gets a clear 405 rather than the static 404.
        .route(
            "/api/agents/{id}",
            axum::routing::any(|| async { StatusCode::METHOD_NOT_ALLOWED }),
        )
        .route("/api/agents/{id}/stop", post(stop_agent))
        .route("/api/agents/{id}/answer", post(answer_agent))
        .route("/api/agents/{id}/pause", post(pause_agent))
        .route("/api/agents/{id}/resume", post(resume_agent))
        .route("/api/agents/{id}/terminal", get(terminal))
        .route("/api/agents/{id}/diff", get(diff))
        .route("/api/agents/{id}/repos", post(add_repo))
        .route("/api/agents/{id}/repo-request/grant", post(grant_repo))
        .route("/api/agents/{id}/repo-request/refuse", post(refuse_repo))
        .route("/api/projects", get(list_projects))
        .route("/api/projects/refresh", post(refresh_projects))
        .route("/api/projects/{id}/archive-plan", get(archive_plan))
        .route("/api/projects/{id}/archive", post(archive_project))
        .route("/api/projects/{id}/fleet", get(fleet_view))
        .route(
            "/api/projects/{id}/breakdown/decide",
            post(decide_breakdown),
        )
        .route("/api/orphans/plan", post(orphan_plan))
        .route("/api/orphans/archive", post(archive_orphan))
        .route("/api/orphans/end-session", post(end_orphan_session))
        .route("/api/personas", get(list_personas).post(create_persona))
        .route(
            "/api/personas/{id}",
            patch(update_persona).delete(delete_persona),
        )
        .route("/api/personas/{id}/check", post(check_persona))
        .route(
            "/api/agent-config",
            get(get_agent_config).put(put_agent_config),
        )
        .route("/api/definitions", get(list_definitions))
        .route(
            "/api/definitions/{kind}/{name}",
            get(get_definition)
                .put(put_definition)
                .delete(reset_definition),
        )
        .route("/api/events", get(events))
        .route("/api/repos", get(repos))
        .route(
            "/api/ui-schema",
            get(|| async { Json(default_ui_schema()) }),
        )
        .route("/api/openapi.json", get(|| async { Json(openapi()) }))
        .route("/internal/hooks/{id}", post(hook))
        .route("/internal/mcp/{id}", post(mcp))
        .fallback(crate::ui::static_handler)
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        // Outermost, so a request the guard refuses is still inside its span.
        .layer(middleware::from_fn(request_span))
        .with_state(state)
}

/// Refuses requests whose Host is not this daemon's loopback address (DNS rebinding) and requests
/// whose Origin is not the same origin (another web page driving the API). Spawning an agent runs code.
async fn guard(State(st): State<AppState>, req: Request, next: Next) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|h| h.to_str().ok());
    let host_ok = host.rsplit_once(':').is_some_and(|(name, port)| {
        matches!(name, "127.0.0.1" | "localhost" | "[::1]") && port == st.port.to_string()
    });
    let origin_ok = origin.is_none_or(|o| o.eq_ignore_ascii_case(&format!("http://{host}")));
    if host_ok && origin_ok {
        next.run(req).await
    } else {
        ApiError(StatusCode::FORBIDDEN, "forbidden host or origin".into()).into_response()
    }
}

/// Puts everything a request logs under one `http` span. Only the path is recorded, never the query string.
async fn request_span(req: Request, next: Next) -> Response {
    let span = tracing::info_span!("http", method = %req.method(), path = %req.uri().path());
    next.run(req).instrument(span).await
}

struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

/// What the user typed was wrong (or could not be cloned), as opposed to the daemon failing.
fn repo_status(e: &RepoError) -> StatusCode {
    match e {
        RepoError::Unrecognised(_)
        | RepoError::NotARepo(_)
        | RepoError::Bare(_)
        | RepoError::InsideProjects(_)
        | RepoError::Clone { .. }
        | RepoError::Credentials(_)
        | RepoError::Ssh(_)
        | RepoError::PlainHttp(_)
        | RepoError::WrongHost { .. }
        | RepoError::OtherPersona { .. } => StatusCode::BAD_REQUEST,
        RepoError::BranchExists { .. } | RepoError::Start { .. } => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// A persona that cannot run a project now: the caller picked it, so it is the request that is wrong.
fn persona_status(e: &PersonaError) -> StatusCode {
    match e {
        PersonaError::Io(_) | PersonaError::Store(_) => StatusCode::INTERNAL_SERVER_ERROR,
        _ => StatusCode::BAD_REQUEST,
    }
}

/// A definition a spawn names that is missing or broken: the request is what is wrong, not the route.
fn fleet_status(e: &FleetError) -> StatusCode {
    match e {
        FleetError::Io { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        _ => StatusCode::BAD_REQUEST,
    }
}

impl From<SpawnError> for ApiError {
    fn from(e: SpawnError) -> Self {
        let status = match &e {
            SpawnError::Repo(r) => repo_status(r),
            SpawnError::Persona(p) => persona_status(p),
            SpawnError::NoPersona | SpawnError::AgentConfig(_) => StatusCode::BAD_REQUEST,
            SpawnError::EmptyPrompt | SpawnError::PromptTooLong => StatusCode::BAD_REQUEST,
            SpawnError::Git(GitError::NotARepo(_)) => StatusCode::BAD_REQUEST,
            SpawnError::OneRepo
            | SpawnError::NoTestCommand(_)
            | SpawnError::SpecialistSpawn
            | SpawnError::NoRoster => StatusCode::BAD_REQUEST,
            SpawnError::Fleet(f) => fleet_status(f),
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError(status, e.to_string())
    }
}

impl From<ServiceError> for ApiError {
    fn from(e: ServiceError) -> Self {
        let status = match &e {
            ServiceError::NotFound | ServiceError::NotAnOrphan => StatusCode::NOT_FOUND,
            ServiceError::NotRunning
            | ServiceError::NotStopped
            | ServiceError::SessionNotArchived
            | ServiceError::SessionStillLive
            | ServiceError::PlanChanged
            | ServiceError::NoPendingRequest
            | ServiceError::RequestPending
            | ServiceError::OneRepo
            | ServiceError::NotAnOrchestrator
            | ServiceError::NoPendingBreakdown
            | ServiceError::BreakdownPending => StatusCode::CONFLICT,
            ServiceError::Breakdown(_) => StatusCode::BAD_REQUEST,
            ServiceError::EmptyText | ServiceError::NoPersona | ServiceError::AgentConfig(_) => {
                StatusCode::BAD_REQUEST
            }
            ServiceError::Repo(r) => repo_status(r),
            ServiceError::Persona(p) => persona_status(p),
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError(status, e.to_string())
    }
}

impl From<PersonaError> for ApiError {
    fn from(e: PersonaError) -> Self {
        let status = match &e {
            PersonaError::NotFound(_) => StatusCode::NOT_FOUND,
            PersonaError::BadName | PersonaError::BadIdentity | PersonaError::BadHost(_) => {
                StatusCode::BAD_REQUEST
            }
            PersonaError::HasProjects(_)
            | PersonaError::NotReady(_)
            | PersonaError::LoginBroken { .. } => StatusCode::CONFLICT,
            PersonaError::Io(_) | PersonaError::Store(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError(status, e.to_string())
    }
}

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

#[derive(serde::Deserialize)]
struct ReposQuery {
    persona: Option<String>,
}

async fn repos(
    State(st): State<AppState>,
    Query(q): Query<ReposQuery>,
) -> Result<Json<RepoList>, ApiError> {
    let rows = st.svc.remembered_repos(q.persona.as_deref()).await?;
    Ok(Json(RepoList {
        repos: rows
            .into_iter()
            .map(|r| RepoInfo {
                name: r.name,
                source: r.source,
                location: redact_url(&r.location),
                persona: r.persona,
                roster: r.roster,
                test_command: r.test_command,
            })
            .collect(),
    }))
}

async fn get_agent_config(State(st): State<AppState>) -> Result<Json<AgentConfig>, ApiError> {
    let dir = st.svc.cfg.agent_config_dir.clone();
    // Files are read and written on a blocking thread, as nothing else here touches the disk on the runtime.
    let c = tokio::task::spawn_blocking(move || agent_config::load_lenient(&dir))
        .await
        .map_err(internal)?
        .map_err(config_error)?;
    Ok(Json(c))
}

async fn put_agent_config(
    State(st): State<AppState>,
    Json(u): Json<AgentConfigUpdate>,
) -> Result<Json<AgentConfig>, ApiError> {
    let dir = st.svc.cfg.agent_config_dir.clone();
    let saved = tokio::task::spawn_blocking(move || {
        agent_config::save(&dir, &u)?;
        agent_config::load_lenient(&dir)
    })
    .await
    .map_err(internal)?;
    Ok(Json(saved.map_err(config_error)?))
}

/// Everything but a disk failure is something the file or the request holds, which the GUI can show.
fn config_error(e: agent_config::AgentConfigError) -> ApiError {
    match e {
        agent_config::AgentConfigError::Io(_) | agent_config::AgentConfigError::Encode(_) => {
            internal(e)
        }
        _ => ApiError(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

impl From<FleetError> for ApiError {
    fn from(e: FleetError) -> Self {
        let status = match &e {
            FleetError::Invalid { .. } => StatusCode::BAD_REQUEST,
            // A built-in with no user copy has nothing at this path to delete.
            FleetError::NotFound { .. } | FleetError::Builtin(_) => StatusCode::NOT_FOUND,
            FleetError::Io { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError(status, e.to_string())
    }
}

/// Definitions are files: read and written on a blocking thread, as the agent config is.
async fn with_defs<T: Send + 'static>(
    st: &AppState,
    f: impl FnOnce(Defs) -> Result<T, FleetError> + Send + 'static,
) -> Result<T, ApiError> {
    let defs = Defs::new(st.svc.cfg.fleet_dir.clone());
    Ok(tokio::task::spawn_blocking(move || f(defs))
        .await
        .map_err(internal)??)
}

async fn list_definitions(State(st): State<AppState>) -> Result<Json<Vec<DefInfo>>, ApiError> {
    Ok(Json(with_defs(&st, |d| Ok(d.list())).await?))
}

async fn get_definition(
    State(st): State<AppState>,
    Path((kind, name)): Path<(DefKind, String)>,
) -> Result<Json<DefFile>, ApiError> {
    Ok(Json(with_defs(&st, move |d| d.read(kind, &name)).await?))
}

async fn put_definition(
    State(st): State<AppState>,
    Path((kind, name)): Path<(DefKind, String)>,
    text: String,
) -> Result<Json<DefInfo>, ApiError> {
    Ok(Json(
        with_defs(&st, move |d| d.write(kind, &name, &text)).await?,
    ))
}

async fn reset_definition(
    State(st): State<AppState>,
    Path((kind, name)): Path<(DefKind, String)>,
) -> Result<StatusCode, ApiError> {
    with_defs(&st, move |d| d.reset(kind, &name)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_personas(State(st): State<AppState>) -> Result<Json<PersonaList>, ApiError> {
    Ok(Json(st.svc.personas().list().await?))
}

async fn create_persona(
    State(st): State<AppState>,
    Json(req): Json<NewPersona>,
) -> Result<Json<Persona>, ApiError> {
    Ok(Json(st.svc.personas().create(req).await?))
}

async fn update_persona(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<PersonaUpdate>,
) -> Result<Json<Persona>, ApiError> {
    Ok(Json(st.svc.personas().update(&id, req).await?))
}

async fn delete_persona(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    st.svc.personas().delete(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Runs the CLIs, so it may take up to their timeout; detached so a client that hangs up cannot cut the
/// adoption of a first login short.
async fn check_persona(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Persona>, ApiError> {
    Ok(Json(
        detached(async move { st.svc.personas().check(&id).await }).await??,
    ))
}

async fn list_agents(State(st): State<AppState>) -> Result<Json<Snapshot>, ApiError> {
    Ok(Json(st.store.snapshot().await.map_err(internal)?))
}

/// Runs `work` on its own task and waits for it. Axum drops a handler whose client hangs up (a timeout, a closed
/// window), and a spawn, attach or stop dropped there would leave a project folder, worktree or branch behind with
/// no card to remove it with; a task runs to the end, undoing its own failures as usual.
async fn detached<T: Send + 'static>(
    work: impl std::future::Future<Output = T> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::spawn(work).await.map_err(internal)
}

async fn spawn_agent(
    State(st): State<AppState>,
    Json(req): Json<SpawnRequest>,
) -> Result<Json<SpawnResponse>, ApiError> {
    Ok(Json(
        detached(async move { st.svc.spawn_reporting(req).await }).await??,
    ))
}

async fn stop_agent(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    detached(async move { st.svc.stop(&AgentId(id)).await }).await??;
    Ok(StatusCode::NO_CONTENT)
}

async fn add_repo(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<AddRepoRequest>,
) -> Result<Json<Workspace>, ApiError> {
    Ok(Json(
        detached(async move { st.svc.attach_repo(&AgentId(id), &req.repo).await }).await??,
    ))
}

async fn grant_repo(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<GrantRepoRequest>,
) -> Result<Json<Workspace>, ApiError> {
    Ok(Json(
        detached(async move { st.svc.grant_repo(&AgentId(id), &req.repo).await }).await??,
    ))
}

async fn refuse_repo(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<RefuseRepoRequest>,
) -> Result<StatusCode, ApiError> {
    detached(async move { st.svc.refuse_repo(&AgentId(id), &req.reason).await }).await??;
    Ok(StatusCode::NO_CONTENT)
}

async fn fleet_view(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<FleetView>, ApiError> {
    Ok(Json(st.svc.fleet_view(&AgentId(id)).await?))
}

async fn decide_breakdown(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<DecideBreakdown>,
) -> Result<StatusCode, ApiError> {
    detached(async move { st.svc.decide_breakdown(&AgentId(id), req).await }).await??;
    Ok(StatusCode::NO_CONTENT)
}

async fn answer_agent(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<AnswerRequest>,
) -> Result<StatusCode, ApiError> {
    st.svc.answer(&AgentId(id), &req.text).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn pause_agent(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(_): Json<ControlRequest>,
) -> Result<StatusCode, ApiError> {
    st.svc.pause(&AgentId(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn resume_agent(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(_): Json<ControlRequest>,
) -> Result<StatusCode, ApiError> {
    st.svc.resume(&AgentId(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn diff(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<fleet_api::DiffReport>, ApiError> {
    Ok(Json(st.svc.diff(&AgentId(id)).await?))
}

/// Served from what the daemon already holds: the cached orphan list and clone-cache size never trigger a walk.
async fn list_projects(State(st): State<AppState>) -> Result<Json<ProjectList>, ApiError> {
    Ok(Json(ProjectList {
        projects: st.store.projects().await.map_err(internal)?,
        orphans: st.svc.orphans(),
        clones_bytes: st.svc.clones_bytes(),
    }))
}

/// Starts the rescan and the measurements and returns at once; the Projects view picks the results up by polling.
async fn refresh_projects(State(st): State<AppState>) -> StatusCode {
    st.svc.request_rescan();
    st.svc.request_measure_all();
    StatusCode::ACCEPTED
}

async fn archive_plan(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ArchivePlan>, ApiError> {
    Ok(Json(
        detached(async move { st.svc.archive_plan(&AgentId(id)).await }).await??,
    ))
}

async fn archive_project(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ArchiveRequest>,
) -> Result<Json<ArchiveReport>, ApiError> {
    Ok(Json(
        detached(async move { st.svc.archive(&AgentId(id), &req.items).await }).await??,
    ))
}

async fn orphan_plan(
    State(st): State<AppState>,
    Json(req): Json<OrphanRequest>,
) -> Result<Json<ArchivePlan>, ApiError> {
    Ok(Json(
        detached(async move { st.svc.orphan_plan(&req.id).await }).await??,
    ))
}

async fn archive_orphan(
    State(st): State<AppState>,
    Json(req): Json<ArchiveOrphanRequest>,
) -> Result<Json<ArchiveReport>, ApiError> {
    Ok(Json(
        detached(async move { st.svc.archive_orphan(&req.id, &req.items).await }).await??,
    ))
}

/// The orphan list is keyed by orphan id; the service ends a session by its name, and re-checks it against tmux.
async fn end_orphan_session(
    State(st): State<AppState>,
    Json(req): Json<OrphanRequest>,
) -> Result<StatusCode, ApiError> {
    let session = st
        .svc
        .orphans()
        .into_iter()
        .find(|o| o.id == req.id && o.kind == OrphanKind::Session)
        .ok_or(ServiceError::NotAnOrphan)?;
    detached(async move { st.svc.end_session(&session.path).await }).await??;
    Ok(StatusCode::NO_CONTENT)
}

async fn hook(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let token = headers.get("x-fleet-token").and_then(|v| v.to_str().ok());
    let now = st.svc.options().clock.now();
    let id = AgentId(id);
    if let IngestOutcome::Applied(events) = ingest(&st.store, &id, token, body, now).await? {
        // An agent that exited on its own is weighed like a stopped one, for the Projects view.
        let exited = events.iter().any(|e| {
            matches!(
                e.kind,
                EventKind::StatusChanged {
                    status: Status::Stopped,
                    ..
                }
            )
        });
        if exited {
            st.svc.request_measure(&id);
        }
    }
    Ok(Json(json!({})))
}

impl From<IngestError> for ApiError {
    fn from(e: IngestError) -> Self {
        match e {
            IngestError::UnknownAgent => ApiError(StatusCode::NOT_FOUND, "unknown agent".into()),
            IngestError::BadToken => ApiError(StatusCode::UNAUTHORIZED, "bad token".into()),
            IngestError::BadBody(e) => ApiError(StatusCode::BAD_REQUEST, e.to_string()),
            IngestError::Store(e) => internal(e),
        }
    }
}

/// The agent's MCP server. A `request_repo` call is answered only once the owner decides, so this response may
/// take many minutes; Claude Code is started with timeouts that allow it (`agents::MCP_HOLD_ENV`).
async fn mcp(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let id = AgentId(id);
    let token = headers.get("x-fleet-token").and_then(|v| v.to_str().ok());
    // A stopped agent's session is gone; whatever still holds its token is answered as if the agent did not exist.
    match st.svc.agent(&id).await {
        Ok(a) if a.status != Status::Stopped => {}
        Ok(_) | Err(ServiceError::NotFound) => return Err(IngestError::UnknownAgent.into()),
        Err(e) => return Err(e.into()),
    }
    authenticate(&st.store, &id, token).await?;
    Ok(match crate::mcp::handle(&st.svc, &id, body).await {
        Value::Null => StatusCode::ACCEPTED.into_response(),
        reply => Json(reply).into_response(),
    })
}

struct StreamCount(Arc<AtomicUsize>);

impl Drop for StreamCount {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

type SseStream = Sse<KeepAliveStream<ReceiverStream<Result<SseEvent, Infallible>>>>;

/// How often a quiet event stream says it is alive. Clients treat three missed ones (45 s) as a dead connection.
const KEEP_ALIVE: Duration = Duration::from_secs(15);

fn snapshot_event(snap: &Snapshot) -> Option<SseEvent> {
    SseEvent::default()
        .event("snapshot")
        .id(snap.seq.to_string())
        .json_data(snap)
        .ok()
}

fn delta_event(ev: &fleet_api::Event) -> Option<SseEvent> {
    SseEvent::default()
        .event("event")
        .id(ev.seq.to_string())
        .json_data(ev)
        .ok()
}

/// `Last-Event-ID` resume: missed events when the log still has them, otherwise one fresh snapshot.
/// Subscribing before reading the catch-up closes the gap between the two.
async fn events(State(st): State<AppState>, headers: HeaderMap) -> SseStream {
    let last: Option<u64> = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    let mut live = st.store.subscribe();
    let keep_alive = st.keep_alive;
    let (tx, rx) = mpsc::channel(256);
    tokio::spawn(async move {
        let mut sent = match catch_up(&st.store, last, &tx).await {
            Some(seq) => seq,
            None => return,
        };
        loop {
            let next = tokio::select! {
                _ = tx.closed() => return,
                r = live.recv() => r,
            };
            match next {
                Ok(ev) if ev.seq > sent => {
                    let Some(e) = delta_event(&ev) else { continue };
                    if tx.send(Ok(e)).await.is_err() {
                        return;
                    }
                    sent = ev.seq;
                }
                Ok(_) => {}
                // Too slow for the broadcast buffer: start over from a snapshot instead of leaving a gap.
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    match catch_up(&st.store, None, &tx).await {
                        Some(seq) => sent = seq,
                        None => return,
                    }
                }
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    });
    // A named event, not the default comment: a browser's EventSource never surfaces comments, so the web UI could
    // not tell a quiet stream from a dead one. The egui client ignores events it does not know.
    Sse::new(ReceiverStream::new(rx)).keep_alive(
        KeepAlive::new()
            .interval(keep_alive)
            .event(SseEvent::default().event("ping")),
    )
}

/// Sends the initial events and returns the highest `seq` delivered, or `None` when the client is gone.
async fn catch_up(
    store: &Store,
    last: Option<u64>,
    tx: &mpsc::Sender<Result<SseEvent, Infallible>>,
) -> Option<u64> {
    if let Some(n) = last {
        if let Ok(Some(events)) = store.events_since(n).await {
            let mut sent = n;
            for ev in events {
                tx.send(Ok(delta_event(&ev)?)).await.ok()?;
                sent = ev.seq;
            }
            return Some(sent);
        }
    }
    let snap = store.snapshot().await.ok()?;
    tx.send(Ok(snapshot_event(&snap)?)).await.ok()?;
    Some(snap.seq)
}

/// Read-only: polls the pane and sends its text whenever it changes. There is deliberately no write route.
async fn terminal(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<SseStream, ApiError> {
    let agent = st
        .store
        .snapshot()
        .await
        .map_err(internal)?
        .agents
        .into_iter()
        .find(|a| a.id.0 == id)
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such agent".into()))?;
    let (tx, rx) = mpsc::channel(8);
    if agent.status == Status::Stopped {
        // No session to poll: the screen saved when it stopped is all there is, sent once, then `closed` as when a
        // live session ends, so the clients stop reconnecting.
        let saved = st
            .store
            .project(&agent.id)
            .await
            .map_err(internal)?
            .and_then(|p| p.last_screen)
            .unwrap_or_else(|| "not saved".into());
        if let Ok(e) = SseEvent::default().event("pane").json_data(&saved) {
            let _ = tx.send(Ok(e)).await;
        }
        let _ = tx
            .send(Ok(SseEvent::default().event("closed").data("")))
            .await;
        return Ok(Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default()));
    }
    st.terminals.fetch_add(1, Ordering::SeqCst);
    let open = st.terminals.clone();
    tokio::spawn(async move {
        let _count = StreamCount(open);
        let mut last: Option<String> = None;
        loop {
            match st.tmux.capture(&agent.tmux_session).await {
                Ok(text) if last.as_deref() != Some(&text) => {
                    let Ok(e) = SseEvent::default().event("pane").json_data(&text) else {
                        return;
                    };
                    if tx.send(Ok(e)).await.is_err() {
                        return;
                    }
                    last = Some(text);
                }
                Ok(_) => {}
                Err(_) => {
                    let _ = tx
                        .send(Ok(SseEvent::default().event("closed").data("")))
                        .await;
                    return;
                }
            }
            // An idle pane never changes, so a gone client is only noticed by watching for it here.
            tokio::select! {
                _ = tx.closed() => return,
                _ = tokio::time::sleep(TERMINAL_POLL) => {}
            }
        }
    });
    Ok(Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default()))
}
