//! The daemon's HTTP surface: JSON commands, SSE event and terminal streams, and the hook ingress.
use crate::agent_config;
use crate::agents::{AgentService, ServiceError, SpawnError};
use crate::fleet::defs::{Defs, FleetError};
use crate::fleet::workflow::{self, WorkflowError};
use crate::git::GitError;
use crate::ingest::{authenticate, ingest, IngestError, IngestOutcome};
use crate::internal::InternalError;
use crate::overrides;
use crate::personas::PersonaError;
use crate::products::ProductError;
use crate::repos::{redact_url, RepoError};
use crate::session::{Output, SessionHost};
use crate::store::{ProductRow, ReleaseRow, RepoRow, Store};
use axum::extract::{Path, Query, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event as SseEvent, KeepAlive, KeepAliveStream, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};
use fleet_api::{
    default_ui_schema, openapi, AddRepoRequest, Agent, AgentConfig, AgentConfigUpdate, AgentId,
    AnswerRequest, ArchiveOrphanRequest, ArchivePlan, ArchiveReport, ArchiveRequest,
    ControlRequest, DecideBreakdown, DefFile, DefInfo, DefKind, EffectiveSettings, EventKind,
    FleetView, GrantRepoRequest, InternalBranches, InternalFile, InternalFiles, InternalHistory,
    NewPersona, NewProduct, NewRelease, OrphanRequest, OverridesPut, Persona, PersonaList,
    PersonaUpdate, Product, ProductList, ProductUpdate, RefuseRepoRequest, Release, ReleaseUpdate,
    RepoFleet, RepoInfo, RepoList, RepoUpsert, SetTaskRelease, Snapshot, SpawnRequest,
    SpawnResponse, Status, StopTaskResponse, Task, TaskList, Workspace,
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

const PREVIEW_EVERY: Duration = Duration::from_millis(100);

#[derive(Clone)]
pub struct AppState {
    svc: Arc<AgentService>,
    store: Arc<Store>,
    sessions: SessionHost,
    port: u16,
    attach: Arc<AttachCmd>,
    terminals: Arc<AtomicUsize>,
    keep_alive: Duration,
    /// Told when a client asks the daemon to stop (POST /api/shutdown).
    shutdown: Arc<tokio::sync::Notify>,
}

impl AppState {
    /// `addr` is the address the listener is bound to; the Host/Origin guard compares against its port, and an
    /// agent's attach command points at it.
    pub fn new(
        svc: Arc<AgentService>,
        store: Arc<Store>,
        sessions: SessionHost,
        addr: SocketAddr,
    ) -> AppState {
        AppState {
            svc,
            store,
            sessions,
            port: addr.port(),
            attach: Arc::new(AttachCmd::new(addr)),
            terminals: Arc::new(AtomicUsize::new(0)),
            keep_alive: KEEP_ALIVE,
            shutdown: Arc::new(tokio::sync::Notify::new()),
        }
    }

    /// How often a quiet event stream pings; tests shorten it.
    #[doc(hidden)]
    pub fn with_keep_alive(mut self, every: Duration) -> AppState {
        self.keep_alive = every;
        self
    }

    /// The `Notify` that `POST /api/shutdown` triggers; `main` waits on it alongside Ctrl-C and SIGTERM.
    pub fn with_shutdown(mut self, notify: Arc<tokio::sync::Notify>) -> AppState {
        self.shutdown = notify;
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
        .route("/api/agents/{id}/attach", get(attach_agent))
        .route("/api/agents/{id}/diff", get(diff))
        .route("/api/agents/{id}/repos", post(add_repo))
        .route("/api/agents/{id}/repo-request/grant", post(grant_repo))
        .route("/api/agents/{id}/repo-request/refuse", post(refuse_repo))
        .route("/api/tasks", get(list_tasks))
        .route("/api/tasks/refresh", post(refresh_tasks))
        .route("/api/tasks/{id}/archive-plan", get(archive_plan))
        .route("/api/tasks/{id}/archive", post(archive_task))
        .route("/api/tasks/{id}/fleet", get(fleet_view))
        .route("/api/tasks/{id}/breakdown/decide", post(decide_breakdown))
        .route("/api/tasks/{id}/stop", post(stop_task))
        .route("/api/tasks/{id}/restart", post(restart_orchestrator))
        .route("/api/orphans/plan", post(orphan_plan))
        .route("/api/orphans/archive", post(archive_orphan))
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
        .route("/api/products", get(products).post(create_product))
        .route(
            "/api/products/{id}",
            patch(update_product).delete(delete_product),
        )
        .route("/api/products/{id}/repos", put(upsert_repo))
        .route("/api/products/{id}/repos/{name}", delete(delete_repo))
        .route("/api/products/{id}/repos/{name}/fleet", put(put_repo_fleet))
        .route("/api/products/{id}/overrides", put(put_product_overrides))
        .route(
            "/api/products/{id}/internal/branches",
            get(internal_branches),
        )
        .route("/api/products/{id}/internal/files", get(internal_files))
        .route("/api/products/{id}/internal/file", get(internal_file))
        .route("/api/products/{id}/internal/history", get(internal_history))
        .route("/api/settings/effective", get(effective_settings))
        .route("/api/products/{id}/releases", post(create_release))
        .route(
            "/api/releases/{id}",
            patch(update_release).delete(delete_release),
        )
        .route("/api/tasks/{id}/release", put(set_task_release))
        .route("/api/tasks/{id}/done", post(mark_done))
        .route("/api/repos", get(repos))
        .route(
            "/api/ui-schema",
            get(|| async { Json(default_ui_schema()) }),
        )
        .route("/api/shutdown", post(shutdown))
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
        | RepoError::InsideTasks(_)
        | RepoError::Clone { .. }
        | RepoError::Credentials(_)
        | RepoError::Ssh(_)
        | RepoError::PlainHttp(_)
        | RepoError::WrongHost { .. }
        | RepoError::NoUrl(_)
        | RepoError::NoPath(_)
        | RepoError::UnknownProduct(_)
        | RepoError::BadName(_)
        | RepoError::NeedPathOrUrl => StatusCode::BAD_REQUEST,
        RepoError::BranchExists { .. } | RepoError::Start { .. } | RepoError::UrlChange(_) => {
            StatusCode::CONFLICT
        }
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// A persona that cannot run a task now: the caller picked it, so it is the request that is wrong.
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
            // A git failure reading the file is the daemon's, not the request's.
            SpawnError::Workflow {
                error: WorkflowError::Git(_),
                ..
            } => StatusCode::INTERNAL_SERVER_ERROR,
            // The remote is down or slow: the request may succeed as is on a retry.
            SpawnError::Workflow {
                error: WorkflowError::Fetch(_),
                ..
            } => StatusCode::BAD_GATEWAY,
            SpawnError::Workflow {
                error: WorkflowError::FetchTimedOut,
                ..
            } => StatusCode::GATEWAY_TIMEOUT,
            SpawnError::OneRepo
            | SpawnError::Workflow { .. }
            | SpawnError::SpecialistSpawn
            | SpawnError::NoRoster
            | SpawnError::ReleaseNotInProduct { .. } => StatusCode::BAD_REQUEST,
            SpawnError::Released(_) => StatusCode::CONFLICT,
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
            | ServiceError::SessionStillLive
            | ServiceError::PlanChanged
            // A deliberate refusal: the caller can act on the text. A real removal failure stays 500.
            | ServiceError::Refused(_)
            | ServiceError::OpenPr(_)
            | ServiceError::NoPendingRequest
            | ServiceError::RequestPending
            | ServiceError::OneRepo
            | ServiceError::NotAnOrchestrator
            | ServiceError::NoPendingBreakdown
            | ServiceError::StaleBreakdown { .. }
            | ServiceError::BreakdownPending
            | ServiceError::Fleet(_) => StatusCode::CONFLICT,
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
            PersonaError::HasTasks(_)
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
    product: Option<String>,
}

async fn repos(
    State(st): State<AppState>,
    Query(q): Query<ReposQuery>,
) -> Result<Json<RepoList>, ApiError> {
    let rows = st.svc.remembered_repos(q.product.as_deref()).await?;
    let mut repos = Vec::with_capacity(rows.len());
    for r in rows {
        repos.push(repo_info(&st, r).await);
    }
    Ok(Json(RepoList { repos }))
}

async fn repo_info(st: &AppState, r: RepoRow) -> RepoInfo {
    let workflow = workflow::state(st.svc.repos.listing_base(&r)).await;
    RepoInfo {
        product: r.product,
        name: r.name,
        path: r.path.map(|p| p.to_string_lossy().into_owned()),
        url: r.url.as_deref().map(redact_url),
        roster: r.roster,
        workflow,
    }
}

fn release_view(r: ReleaseRow) -> Release {
    Release {
        id: r.id,
        product: r.product,
        version: r.version,
        start: r.start,
        target: r.target,
        released_at: r.released_at,
    }
}

async fn product_view(st: &AppState, p: ProductRow) -> Result<Product, ApiError> {
    let tasks = st
        .store
        .product_task_counts()
        .await
        .map_err(internal)?
        .get(&p.id)
        .copied()
        .unwrap_or(0);
    let mut repos = Vec::new();
    for r in st.svc.remembered_repos(Some(&p.id)).await? {
        repos.push(repo_info(st, r).await);
    }
    let releases = st
        .store
        .releases(&p.id)
        .await
        .map_err(internal)?
        .into_iter()
        .map(release_view)
        .collect();
    let stored = st.store.product_overrides(&p.id).await.map_err(internal)?;
    // The definitions are files: only read them when there is an entry to check against them.
    let overrides = if stored.is_empty() {
        stored
    } else {
        with_defs(st, move |d| Ok(overrides::annotate(stored, &d))).await?
    };
    Ok(Product {
        overrides,
        tasks,
        id: p.id,
        name: p.name,
        default_persona: p.default_persona,
        repos,
        releases,
    })
}

async fn products(State(st): State<AppState>) -> Result<Json<ProductList>, ApiError> {
    let mut products = Vec::new();
    for p in st.store.products().await.map_err(internal)? {
        products.push(product_view(&st, p).await?);
    }
    Ok(Json(ProductList { products }))
}

impl From<ProductError> for ApiError {
    fn from(e: ProductError) -> Self {
        let status = match &e {
            ProductError::NotFound(_)
            | ProductError::NoRelease(_)
            | ProductError::NoRepo(_)
            | ProductError::NoTask => StatusCode::NOT_FOUND,
            ProductError::BadName
            | ProductError::NoPersona(_)
            | ProductError::BadVersion
            | ProductError::BadDate(_)
            | ProductError::TargetBeforeStart
            | ProductError::OtherProductRelease { .. } => StatusCode::BAD_REQUEST,
            ProductError::HasTasks(_)
            | ProductError::RepoInUse(_)
            | ProductError::Conflict(_)
            | ProductError::Finished
            | ProductError::Released(_) => StatusCode::CONFLICT,
            ProductError::Repo(r) => repo_status(r),
            ProductError::Store(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError(status, e.to_string())
    }
}

async fn create_product(
    State(st): State<AppState>,
    Json(req): Json<NewProduct>,
) -> Result<Json<Product>, ApiError> {
    let row = st.svc.products().create(req).await?;
    Ok(Json(product_view(&st, row).await?))
}

async fn update_product(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ProductUpdate>,
) -> Result<Json<Product>, ApiError> {
    let row = st.svc.products().update(&id, req).await?;
    Ok(Json(product_view(&st, row).await?))
}

async fn delete_product(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    st.svc.products().delete(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn put_product_overrides(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<OverridesPut>,
) -> Result<Json<Product>, ApiError> {
    let row = st
        .store
        .product(&id)
        .await
        .map_err(internal)?
        .ok_or_else(|| ProductError::NotFound(id.clone()))?;
    let entries = req.overrides;
    let entries = with_defs(&st, move |d| {
        Ok((overrides::validate(&entries, &d), entries))
    })
    .await?;
    let (checked, entries) = entries;
    checked.map_err(|m| ApiError(StatusCode::BAD_REQUEST, m))?;
    st.store
        .set_product_overrides(&id, &entries)
        .await
        .map_err(internal)?;
    Ok(Json(product_view(&st, row).await?))
}

impl From<InternalError> for ApiError {
    fn from(e: InternalError) -> Self {
        let status = match e {
            InternalError::NotFound(_) => StatusCode::NOT_FOUND,
            InternalError::BadPath(_) | InternalError::TooLarge | InternalError::NotText => {
                StatusCode::BAD_REQUEST
            }
            InternalError::Git(_) | InternalError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError(status, e.to_string())
    }
}

#[derive(serde::Deserialize)]
struct InternalQuery {
    branch: Option<String>,
    path: Option<String>,
    limit: Option<usize>,
}

impl InternalQuery {
    fn branch(&self) -> &str {
        self.branch.as_deref().unwrap_or("main")
    }
}

/// The product must be in the store: a missing repo dir alone cannot tell an unknown product from one that
/// predates internal resources.
async fn known_product(st: &AppState, id: &str) -> Result<(), ApiError> {
    if st.store.product(id).await.map_err(internal)?.is_none() {
        return Err(ProductError::NotFound(id.to_string()).into());
    }
    Ok(())
}

async fn internal_branches(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<InternalBranches>, ApiError> {
    known_product(&st, &id).await?;
    let branches = match st.svc.internal().branches(&id).await {
        Ok(b) => b,
        // Created before this feature: the first task makes the repo, a read never does.
        Err(InternalError::NotFound(_)) => Vec::new(),
        Err(e) => return Err(e.into()),
    };
    Ok(Json(InternalBranches { branches }))
}

async fn internal_files(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<InternalQuery>,
) -> Result<Json<InternalFiles>, ApiError> {
    known_product(&st, &id).await?;
    let files = st.svc.internal().files(&id, q.branch()).await?;
    Ok(Json(InternalFiles {
        branch: q.branch().to_string(),
        files,
    }))
}

async fn internal_file(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<InternalQuery>,
) -> Result<Json<InternalFile>, ApiError> {
    known_product(&st, &id).await?;
    let path = q
        .path
        .clone()
        .ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "path is required".into()))?;
    let text = st.svc.internal().read(&id, q.branch(), &path).await?;
    Ok(Json(InternalFile {
        branch: q.branch().to_string(),
        path,
        text,
    }))
}

async fn internal_history(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<InternalQuery>,
) -> Result<Json<InternalHistory>, ApiError> {
    known_product(&st, &id).await?;
    let commits = st
        .svc
        .internal()
        .history(&id, q.branch(), q.path.as_deref(), q.limit.unwrap_or(50))
        .await?;
    Ok(Json(InternalHistory { commits }))
}

#[derive(serde::Deserialize)]
struct EffectiveQuery {
    product: Option<String>,
}

async fn effective_settings(
    State(st): State<AppState>,
    Query(q): Query<EffectiveQuery>,
) -> Result<Json<EffectiveSettings>, ApiError> {
    let entries = match &q.product {
        Some(id) => {
            if st.store.product(id).await.map_err(internal)?.is_none() {
                return Err(ProductError::NotFound(id.clone()).into());
            }
            st.store.product_overrides(id).await.map_err(internal)?
        }
        None => Vec::new(),
    };
    let global = st.svc.cfg.limits;
    Ok(Json(
        with_defs(&st, move |d| {
            Ok(overrides::effective(&global, &d, &entries))
        })
        .await?,
    ))
}

async fn upsert_repo(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<RepoUpsert>,
) -> Result<Json<RepoInfo>, ApiError> {
    let row = st
        .svc
        .products()
        .upsert_repo(&id, &req.name, req.path.as_deref(), req.url.as_deref())
        .await?;
    Ok(Json(repo_info(&st, row).await))
}

async fn delete_repo(
    State(st): State<AppState>,
    Path((id, name)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    st.svc.products().delete_repo(&id, &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn create_release(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<NewRelease>,
) -> Result<Json<Release>, ApiError> {
    Ok(Json(release_view(
        st.svc.products().create_release(&id, req).await?,
    )))
}

async fn update_release(
    State(st): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<ReleaseUpdate>,
) -> Result<Json<Release>, ApiError> {
    Ok(Json(release_view(
        st.svc.products().update_release(id, req).await?,
    )))
}

async fn delete_release(
    State(st): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    st.svc.products().delete_release(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_task_release(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<SetTaskRelease>,
) -> Result<Json<Task>, ApiError> {
    Ok(Json(
        st.svc
            .products()
            .set_task_release(&AgentId(id), req.release)
            .await?,
    ))
}

async fn put_repo_fleet(
    State(st): State<AppState>,
    Path((product, name)): Path<(String, String)>,
    Json(f): Json<RepoFleet>,
) -> Result<Json<RepoInfo>, ApiError> {
    let missing = || ApiError(StatusCode::NOT_FOUND, format!("no repo named `{name}`"));
    if st
        .store
        .repo(&product, &name)
        .await
        .map_err(internal)?
        .is_none()
    {
        return Err(missing());
    }
    let roster = f.roster.clone();
    // A missing specialist is the request's fault here, not a missing route: 400, as for a spawn.
    let defs = Defs::new(st.svc.cfg.fleet_dir.clone());
    tokio::task::spawn_blocking(move || {
        roster.iter().try_for_each(|n| defs.specialist(n).map(drop))
    })
    .await
    .map_err(internal)?
    .map_err(|e| ApiError(fleet_status(&e), e.to_string()))?;
    st.store
        .set_repo_roster(&product, &name, &f.roster)
        .await
        .map_err(internal)?;
    let row = st
        .store
        .repo(&product, &name)
        .await
        .map_err(internal)?
        .ok_or_else(missing)?;
    Ok(Json(repo_info(&st, row).await))
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
    let mut snap = st.store.board().await.map_err(internal)?;
    with_attach_snapshot(&st.attach, &mut snap);
    Ok(Json(snap))
}

/// The command that attaches a terminal to an agent: `fleet-attach` beside the daemon's own executable, pointed at
/// the address the daemon listens on. Worked out once; only the agent id varies.
struct AttachCmd {
    exe: String,
    url: String,
}

impl AttachCmd {
    fn new(addr: SocketAddr) -> AttachCmd {
        let exe = std::env::current_exe()
            .ok()
            .and_then(|p| {
                p.parent()
                    .map(|d| d.join(format!("fleet-attach{}", std::env::consts::EXE_SUFFIX)))
            })
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "fleet-attach".into());
        // SocketAddr's Display brackets an IPv6 host, as a URL needs.
        AttachCmd {
            exe,
            url: format!("http://{addr}"),
        }
    }

    fn argv(&self, id: &AgentId) -> Vec<String> {
        vec![
            self.exe.clone(),
            "--url".into(),
            self.url.clone(),
            id.0.clone(),
        ]
    }
}

/// Fills `attach` on the way out. It is derived from the address and the status, so it is never stored: a stored
/// one would go stale on a stop or a daemon on another port.
fn with_attach(cmd: &AttachCmd, a: &mut Agent) {
    a.attach = (a.status != Status::Stopped).then(|| cmd.argv(&a.id));
}

fn with_attach_snapshot(cmd: &AttachCmd, snap: &mut Snapshot) {
    snap.agents.iter_mut().for_each(|a| with_attach(cmd, a));
}

fn with_attach_event(cmd: &AttachCmd, ev: &mut fleet_api::Event) {
    if let EventKind::AgentCreated { agent } = &mut ev.kind {
        with_attach(cmd, agent);
    }
}

/// Runs `work` on its own task and waits for it. Axum drops a handler whose client hangs up (a timeout, a closed
/// window), and a spawn, attach or stop dropped there would leave a task folder, worktree or branch behind with
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
    let attach = st.attach.clone();
    let mut res = detached(async move { st.svc.spawn_reporting(req).await }).await??;
    with_attach(&attach, &mut res.agent);
    Ok(Json(res))
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

async fn stop_task(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<StopTaskResponse>, ApiError> {
    let stopped = detached(async move { st.svc.stop_task(&AgentId(id)).await }).await??;
    Ok(Json(StopTaskResponse { stopped }))
}

async fn restart_orchestrator(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Agent>, ApiError> {
    let svc = st.svc.clone();
    let agent = detached(async move { svc.restart_orchestrator(&AgentId(id)).await }).await??;
    // The new lead inherits a pending breakdown, so it comes back as the board will show it.
    let mut agent = st.store.view(agent).await.map_err(internal)?;
    with_attach(&st.attach, &mut agent);
    Ok(Json(agent))
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
async fn list_tasks(State(st): State<AppState>) -> Result<Json<TaskList>, ApiError> {
    Ok(Json(TaskList {
        tasks: st.store.tasks().await.map_err(internal)?,
        orphans: st.svc.orphans(),
        clones_bytes: st.svc.clones_bytes(),
    }))
}

/// Starts the rescan, the measurements and a PR check, and returns at once; the Tasks view picks the results up by
/// polling.
async fn refresh_tasks(State(st): State<AppState>) -> StatusCode {
    st.svc.request_rescan();
    st.svc.request_measure_all();
    st.svc.request_merge_check(None);
    StatusCode::ACCEPTED
}

async fn mark_done(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Task>, ApiError> {
    Ok(Json(st.svc.mark_done(&AgentId(id)).await?))
}

async fn archive_plan(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ArchivePlan>, ApiError> {
    Ok(Json(
        detached(async move { st.svc.archive_plan(&AgentId(id)).await }).await??,
    ))
}

async fn archive_task(
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

/// Stops the daemon the way SIGTERM does, for clients on systems without signals. Behind the same Host/Origin guard as
/// every route, so only a local, non-browser caller (or the daemon's own page) gets here.
async fn shutdown(State(st): State<AppState>) -> StatusCode {
    tracing::info!("shutdown requested over HTTP");
    // `notify_one` keeps a permit when nobody waits yet, so a request racing start-up is not lost.
    st.shutdown.notify_one();
    StatusCode::ACCEPTED
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
        // An agent that exited on its own is weighed like a stopped one, for the Tasks view.
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
        let mut sent = match catch_up(&st.store, &st.attach, last, &tx).await {
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
                    let mut ev = ev;
                    with_attach_event(&st.attach, &mut ev);
                    let Some(e) = delta_event(&ev) else { continue };
                    if tx.send(Ok(e)).await.is_err() {
                        return;
                    }
                    sent = ev.seq;
                }
                Ok(_) => {}
                // Too slow for the broadcast buffer: start over from a snapshot instead of leaving a gap.
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    match catch_up(&st.store, &st.attach, None, &tx).await {
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
    attach: &AttachCmd,
    last: Option<u64>,
    tx: &mpsc::Sender<Result<SseEvent, Infallible>>,
) -> Option<u64> {
    if let Some(n) = last {
        if let Ok(Some(events)) = store.events_since(n).await {
            let mut sent = n;
            for mut ev in events {
                with_attach_event(attach, &mut ev);
                tx.send(Ok(delta_event(&ev)?)).await.ok()?;
                sent = ev.seq;
            }
            return Some(sent);
        }
    }
    let mut snap = store.board().await.ok()?;
    with_attach_snapshot(attach, &mut snap);
    tx.send(Ok(snapshot_event(&snap)?)).await.ok()?;
    Some(snap.seq)
}

/// Refused before the upgrade when there is no live session to attach to: the upgrade only happens when the
/// handler returns it.
async fn attach_agent(
    State(st): State<AppState>,
    Path(id): Path<String>,
    ws: axum::extract::ws::WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let agent = st.svc.agent(&AgentId(id)).await?;
    if agent.status == Status::Stopped || !st.sessions.alive(&agent.session) {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "agent is not running".into(),
        ));
    }
    Ok(crate::attach::upgrade(
        ws,
        st.sessions.clone(),
        agent.session,
    ))
}

/// Read-only: sends the screen's text when the agent draws, at most every `PREVIEW_EVERY`. There is deliberately no write route.
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
        // No session to watch: the screen saved when it stopped is all there is, sent once, then `closed` as when a
        // live session ends, so the clients stop reconnecting.
        let saved = st
            .store
            .task(&agent.id)
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
        // A running agent without a session is between a daemon start and its resume: the stream ends without
        // `closed`, so the clients reconnect and find the resumed session. Once Stopped, a reconnect gets `closed`.
        let Ok(mut rx) = st.sessions.watch(&agent.session) else {
            return;
        };
        let mut last: Option<String> = None;
        loop {
            match st.sessions.screen_text(&agent.session) {
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
                Err(_) => return,
            }
            // Waits for output, then lets more of it land: at most one pane per PREVIEW_EVERY. A lagged receiver is
            // fine, the next screen read shows the latest state.
            tokio::select! {
                _ = tx.closed() => return,
                r = rx.recv() => {
                    if matches!(r, Ok(Output::Exited(_)) | Err(broadcast::error::RecvError::Closed)) {
                        let _ = tx
                            .send(Ok(SseEvent::default().event("closed").data("")))
                            .await;
                        return;
                    }
                    tokio::time::sleep(PREVIEW_EVERY).await;
                    while rx.try_recv().is_ok() {}
                }
            }
        }
    });
    Ok(Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workflow_status(error: WorkflowError) -> StatusCode {
        ApiError::from(SpawnError::Workflow {
            repo: "core".into(),
            error,
        })
        .0
    }

    #[test]
    fn workflow_fetch_failures_are_gateway_errors() {
        assert_eq!(
            workflow_status(WorkflowError::Fetch("fatal: unreachable".into())),
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(
            workflow_status(WorkflowError::FetchTimedOut),
            StatusCode::GATEWAY_TIMEOUT
        );
        assert_eq!(
            workflow_status(WorkflowError::Missing {
                base: "main".into()
            }),
            StatusCode::BAD_REQUEST
        );
    }
}
