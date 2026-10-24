//! The daemon's HTTP surface: JSON commands, SSE event and terminal streams, and the hook ingress.
use crate::agents::{AgentService, ServiceError, SpawnError};
use crate::git::GitError;
use crate::ingest::{ingest, IngestError};
use crate::store::Store;
use crate::tmux::Tmux;
use axum::extract::{Path, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event as SseEvent, KeepAlive, KeepAliveStream, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use fleet_api::{default_ui_schema, openapi, AgentId, AnswerRequest, ControlRequest, RepoList, Snapshot, SpawnRequest};
use serde_json::{json, Value};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::ReceiverStream;

const TERMINAL_POLL: Duration = Duration::from_millis(500);

#[derive(Clone)]
pub struct AppState {
    svc: Arc<AgentService>,
    store: Arc<Store>,
    tmux: Tmux,
    port: u16,
    terminals: Arc<AtomicUsize>,
}

impl AppState {
    /// `port` is the port the listener is bound to; the Host/Origin guard compares against it.
    pub fn new(svc: Arc<AgentService>, store: Arc<Store>, tmux: Tmux, port: u16) -> AppState {
        AppState { svc, store, tmux, port, terminals: Arc::new(AtomicUsize::new(0)) }
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
        .route("/api/events", get(events))
        .route("/api/repos", get(repos))
        .route("/api/ui-schema", get(|| async { Json(default_ui_schema()) }))
        .route("/api/openapi.json", get(|| async { Json(openapi()) }))
        .route("/internal/hooks/{id}", post(hook))
        .fallback(crate::ui::static_handler)
        .layer(middleware::from_fn_with_state(state.clone(), guard))
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

struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl From<SpawnError> for ApiError {
    fn from(e: SpawnError) -> Self {
        let status = match &e {
            SpawnError::UnknownRepo(_) | SpawnError::EmptyPrompt | SpawnError::PromptTooLong => StatusCode::BAD_REQUEST,
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
            ServiceError::NotRunning => StatusCode::CONFLICT,
            ServiceError::EmptyText => StatusCode::BAD_REQUEST,
            ServiceError::UnknownRepo(_) => StatusCode::CONFLICT,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError(status, e.to_string())
    }
}

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

async fn repos(State(st): State<AppState>) -> Json<RepoList> {
    Json(RepoList { repos: st.svc.config().repos.iter().map(|r| r.name.clone()).collect() })
}

async fn list_agents(State(st): State<AppState>) -> Result<Json<Snapshot>, ApiError> {
    Ok(Json(st.store.snapshot().await.map_err(internal)?))
}

async fn spawn_agent(State(st): State<AppState>, Json(req): Json<SpawnRequest>) -> Result<Json<fleet_api::Agent>, ApiError> {
    Ok(Json(st.svc.spawn(req).await?))
}

async fn kill_agent(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    st.svc.kill(&AgentId(id)).await?;
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
        Err(IngestError::UnknownAgent) => Err(ApiError(StatusCode::NOT_FOUND, "unknown agent".into())),
        Err(IngestError::BadToken) => Err(ApiError(StatusCode::UNAUTHORIZED, "bad token".into())),
        Err(IngestError::BadBody(e)) => Err(ApiError(StatusCode::BAD_REQUEST, e.to_string())),
        Err(IngestError::Store(e)) => Err(internal(e)),
    }
}

struct StreamCount(Arc<AtomicUsize>);

impl Drop for StreamCount {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

type SseStream = Sse<KeepAliveStream<ReceiverStream<Result<SseEvent, Infallible>>>>;

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
    Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default())
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
