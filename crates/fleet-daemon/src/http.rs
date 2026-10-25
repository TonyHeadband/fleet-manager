//! The daemon's HTTP surface: JSON commands, SSE event and terminal streams, and the hook ingress.
use crate::agents::{AgentService, ServiceError, SpawnError};
use crate::git::GitError;
use crate::ingest::{authenticate, ingest, IngestError};
use crate::repos::{redact_url, RepoError};
use crate::store::Store;
use crate::tmux::Tmux;
use axum::extract::{Path, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event as SseEvent, KeepAlive, KeepAliveStream, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use fleet_api::{
    default_ui_schema, openapi, AddRepoRequest, AgentId, AnswerRequest, ControlRequest, GrantRepoRequest, KillReport,
    RefuseRepoRequest, RepoInfo, RepoList, Snapshot, SpawnRequest, SpawnResponse, Workspace,
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
        AppState { svc, store, tmux, port, terminals: Arc::new(AtomicUsize::new(0)), keep_alive: KEEP_ALIVE }
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
        .route("/api/agents/{id}", axum::routing::delete(kill_agent))
        .route("/api/agents/{id}/answer", post(answer_agent))
        .route("/api/agents/{id}/pause", post(pause_agent))
        .route("/api/agents/{id}/resume", post(resume_agent))
        .route("/api/agents/{id}/terminal", get(terminal))
        .route("/api/agents/{id}/diff", get(diff))
        .route("/api/agents/{id}/repos", post(add_repo))
        .route("/api/agents/{id}/repo-request/grant", post(grant_repo))
        .route("/api/agents/{id}/repo-request/refuse", post(refuse_repo))
        .route("/api/events", get(events))
        .route("/api/repos", get(repos))
        .route("/api/ui-schema", get(|| async { Json(default_ui_schema()) }))
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
    let host = req.headers().get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("");
    let origin = req.headers().get(header::ORIGIN).and_then(|h| h.to_str().ok());
    let host_ok = host
        .rsplit_once(':')
        .is_some_and(|(name, port)| matches!(name, "127.0.0.1" | "localhost" | "[::1]") && port == st.port.to_string());
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
        | RepoError::Credentials(_) => StatusCode::BAD_REQUEST,
        RepoError::BranchExists { .. } | RepoError::Start { .. } => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

impl From<SpawnError> for ApiError {
    fn from(e: SpawnError) -> Self {
        let status = match &e {
            SpawnError::Repo(r) => repo_status(r),
            SpawnError::EmptyPrompt | SpawnError::PromptTooLong => StatusCode::BAD_REQUEST,
            SpawnError::Git(GitError::NotARepo(_)) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError(status, e.to_string())
    }
}

impl From<ServiceError> for ApiError {
    fn from(e: ServiceError) -> Self {
        let status = match &e {
            ServiceError::NotFound => StatusCode::NOT_FOUND,
            ServiceError::NotRunning | ServiceError::NoPendingRequest | ServiceError::RequestPending => StatusCode::CONFLICT,
            ServiceError::EmptyText => StatusCode::BAD_REQUEST,
            ServiceError::Repo(r) => repo_status(r),
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError(status, e.to_string())
    }
}

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

async fn repos(State(st): State<AppState>) -> Result<Json<RepoList>, ApiError> {
    let rows = st.svc.remembered_repos().await?;
    Ok(Json(RepoList { repos: rows.into_iter().map(|r| RepoInfo { name: r.name, source: r.source, location: redact_url(&r.location) }).collect() }))
}

async fn list_agents(State(st): State<AppState>) -> Result<Json<Snapshot>, ApiError> {
    Ok(Json(st.store.snapshot().await.map_err(internal)?))
}

/// Runs `work` on its own task and waits for it. Axum drops a handler whose client hangs up (a timeout, a closed
/// window), and a spawn, attach or kill stopped there would leave a project folder, worktree or branch behind with
/// no card to remove it with; a task runs to the end, undoing its own failures as usual.
async fn detached<T: Send + 'static>(work: impl std::future::Future<Output = T> + Send + 'static) -> Result<T, ApiError> {
    tokio::spawn(work).await.map_err(internal)
}

async fn spawn_agent(State(st): State<AppState>, Json(req): Json<SpawnRequest>) -> Result<Json<SpawnResponse>, ApiError> {
    Ok(Json(detached(async move { st.svc.spawn_reporting(req).await }).await??))
}

async fn kill_agent(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<KillReport>, ApiError> {
    Ok(Json(detached(async move { st.svc.kill(&AgentId(id)).await }).await??))
}

async fn add_repo(State(st): State<AppState>, Path(id): Path<String>, Json(req): Json<AddRepoRequest>) -> Result<Json<Workspace>, ApiError> {
    Ok(Json(detached(async move { st.svc.attach_repo(&AgentId(id), &req.repo).await }).await??))
}

async fn grant_repo(State(st): State<AppState>, Path(id): Path<String>, Json(req): Json<GrantRepoRequest>) -> Result<Json<Workspace>, ApiError> {
    Ok(Json(detached(async move { st.svc.grant_repo(&AgentId(id), &req.repo).await }).await??))
}

async fn refuse_repo(State(st): State<AppState>, Path(id): Path<String>, Json(req): Json<RefuseRepoRequest>) -> Result<StatusCode, ApiError> {
    detached(async move { st.svc.refuse_repo(&AgentId(id), &req.reason).await }).await??;
    Ok(StatusCode::NO_CONTENT)
}

async fn answer_agent(State(st): State<AppState>, Path(id): Path<String>, Json(req): Json<AnswerRequest>) -> Result<StatusCode, ApiError> {
    st.svc.answer(&AgentId(id), &req.text).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn pause_agent(State(st): State<AppState>, Path(id): Path<String>, Json(_): Json<ControlRequest>) -> Result<StatusCode, ApiError> {
    st.svc.pause(&AgentId(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn resume_agent(State(st): State<AppState>, Path(id): Path<String>, Json(_): Json<ControlRequest>) -> Result<StatusCode, ApiError> {
    st.svc.resume(&AgentId(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn diff(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<fleet_api::DiffReport>, ApiError> {
    Ok(Json(st.svc.diff(&AgentId(id)).await?))
}

async fn hook(State(st): State<AppState>, Path(id): Path<String>, headers: HeaderMap, Json(body): Json<Value>) -> Result<Json<Value>, ApiError> {
    let token = headers.get("x-fleet-token").and_then(|v| v.to_str().ok());
    let now = st.svc.options().clock.now();
    match ingest(&st.store, &AgentId(id), token, body, now).await {
        Ok(_) => Ok(Json(json!({}))),
        Err(e) => Err(e.into()),
    }
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
async fn mcp(State(st): State<AppState>, Path(id): Path<String>, headers: HeaderMap, Json(body): Json<Value>) -> Result<Response, ApiError> {
    let id = AgentId(id);
    let token = headers.get("x-fleet-token").and_then(|v| v.to_str().ok());
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
    SseEvent::default().event("snapshot").id(snap.seq.to_string()).json_data(snap).ok()
}

fn delta_event(ev: &fleet_api::Event) -> Option<SseEvent> {
    SseEvent::default().event("event").id(ev.seq.to_string()).json_data(ev).ok()
}

/// `Last-Event-ID` resume: missed events when the log still has them, otherwise one fresh snapshot.
/// Subscribing before reading the catch-up closes the gap between the two.
async fn events(State(st): State<AppState>, headers: HeaderMap) -> SseStream {
    let last: Option<u64> = headers.get("last-event-id").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok());
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
                Err(broadcast::error::RecvError::Lagged(_)) => match catch_up(&st.store, None, &tx).await {
                    Some(seq) => sent = seq,
                    None => return,
                },
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    });
    // A named event, not the default comment: a browser's EventSource never surfaces comments, so the web UI could
    // not tell a quiet stream from a dead one. The egui client ignores events it does not know.
    Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::new().interval(keep_alive).event(SseEvent::default().event("ping")))
}

/// Sends the initial events and returns the highest `seq` delivered, or `None` when the client is gone.
async fn catch_up(store: &Store, last: Option<u64>, tx: &mpsc::Sender<Result<SseEvent, Infallible>>) -> Option<u64> {
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
async fn terminal(State(st): State<AppState>, Path(id): Path<String>) -> Result<SseStream, ApiError> {
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
    st.terminals.fetch_add(1, Ordering::SeqCst);
    let open = st.terminals.clone();
    tokio::spawn(async move {
        let _count = StreamCount(open);
        let mut last: Option<String> = None;
        loop {
            match st.tmux.capture(&agent.tmux_session).await {
                Ok(text) if last.as_deref() != Some(&text) => {
                    let Ok(e) = SseEvent::default().event("pane").json_data(&text) else { return };
                    if tx.send(Ok(e)).await.is_err() {
                        return;
                    }
                    last = Some(text);
                }
                Ok(_) => {}
                Err(_) => {
                    let _ = tx.send(Ok(SseEvent::default().event("closed").data(""))).await;
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
