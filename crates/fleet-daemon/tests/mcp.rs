mod common;

use common::net::*;
use common::*;
use fleet_api::{
    Actor, Agent, AgentId, BlockedReason, RepoRequest, SpawnRequest, Status, StopCause, TaskState,
};
use fleet_daemon::agents::{AgentService, RequestGate, ServiceError};
use fleet_daemon::http::{bind_loopback, router, AppState};
use fleet_daemon::ingest::ingest;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

struct Server {
    addr: SocketAddr,
    fx: Fx,
}

async fn serve_with(fx: Fx, svc: Arc<AgentService>) -> Server {
    let listener = bind_loopback("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = AppState::new(svc, fx.store.clone(), fx.sessions.clone(), addr);
    tokio::spawn(async move { axum::serve(listener, router(state)).await });
    Server { addr, fx }
}

async fn start() -> Server {
    let fx = fixture().await;
    let svc = fx.svc.clone();
    serve_with(fx, svc).await
}

async fn spawn(srv: &Server, prompt: &str) -> Agent {
    let req = SpawnRequest {
        actor: Actor("local".into()),
        product: PRODUCT.into(),
        release: None,
        repos: vec![pick(&srv.fx, "core")],
        prompt: prompt.into(),
        slug: None,
        persona: PERSONA.into(),
        kind: fleet_api::AgentKind::Solo,
        role: None,
    };
    let a = srv.fx.svc.spawn(req).await.unwrap();
    assert!(pane_contains(&srv.fx, &a.id, "ready").await);
    a
}

async fn token(srv: &Server, id: &AgentId) -> String {
    srv.fx.store.agent_token(id).await.unwrap().unwrap()
}

/// One JSON-RPC message to the agent's MCP endpoint; returns the HTTP status and the parsed body.
async fn rpc_at(addr: SocketAddr, id: &AgentId, token: &str, body: &Value) -> (u16, Value) {
    let (status, text) = http(
        addr,
        "POST",
        &format!("/internal/mcp/{}", id.0),
        &[("X-Fleet-Token", token)],
        Some(body),
    )
    .await;
    (
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

fn call(name: &str, args: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": name, "arguments": args}})
}

/// The tool result's text and isError flag.
fn result_of(v: &Value) -> (String, bool) {
    let r = &v["result"];
    (
        r["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("no text in {v}"))
            .to_string(),
        r["isError"].as_bool().unwrap(),
    )
}

/// Starts a held `request_repo` call in the background.
fn request_in_background(
    srv: &Server,
    id: &AgentId,
    token: &str,
    repo: &str,
) -> tokio::task::JoinHandle<(u16, Value)> {
    let (addr, id, token, body) = (
        srv.addr,
        id.clone(),
        token.to_string(),
        call("request_repo", json!({"repo": repo, "why": "need it"})),
    );
    tokio::spawn(async move { rpc_at(addr, &id, &token, &body).await })
}

async fn wait_pending(fx: &Fx, id: &AgentId) {
    assert!(
        wait_until(|| async { agent(fx, id).await.repo_request.is_some() }).await,
        "request never became pending"
    );
}

async fn grant(srv: &Server, id: &AgentId, repo: &str) -> (u16, Value) {
    json_of(
        srv.addr,
        "POST",
        &format!("/api/agents/{}/repo-request/grant", id.0),
        Some(&json!({"actor": "local", "repo": repo})),
    )
    .await
}

async fn refuse(srv: &Server, id: &AgentId, reason: &str) -> (u16, Value) {
    json_of(
        srv.addr,
        "POST",
        &format!("/api/agents/{}/repo-request/refuse", id.0),
        Some(&json!({"actor": "local", "reason": reason})),
    )
    .await
}

fn tools_repo(srv: &Server) -> PathBuf {
    make_repo(&srv.fx._tmp.path().join("tools"))
}

#[tokio::test]
async fn mcp_rejects_a_bad_token() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    let path = format!("/internal/mcp/{}", a.id.0);
    assert_eq!(
        http(
            srv.addr,
            "POST",
            &path,
            &[("X-Fleet-Token", "wrong")],
            Some(&body)
        )
        .await
        .0,
        401
    );
    assert_eq!(http(srv.addr, "POST", &path, &[], Some(&body)).await.0, 401);
    let t = token(&srv, &a.id).await;
    assert_eq!(
        http(
            srv.addr,
            "POST",
            "/internal/mcp/nobody",
            &[("X-Fleet-Token", &t)],
            Some(&body)
        )
        .await
        .0,
        404
    );
    assert_eq!(rpc_at(srv.addr, &a.id, &t, &body).await.0, 200);
}

#[tokio::test]
async fn initialize_echoes_the_protocol_and_notifications_get_202() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    let t = token(&srv, &a.id).await;
    let init = json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": "2025-11-25", "capabilities": {}}});
    let (status, v) = rpc_at(srv.addr, &a.id, &t, &init).await;
    assert_eq!(status, 200, "{v}");
    assert_eq!(
        (v["id"].clone(), v["result"]["protocolVersion"].clone()),
        (json!(0), json!("2025-11-25"))
    );
    assert_eq!(v["result"]["capabilities"], json!({"tools": {}}));
    assert_eq!(v["result"]["serverInfo"]["name"], "fleet-manager");
    let (status, body) = http(
        srv.addr,
        "POST",
        &format!("/internal/mcp/{}", a.id.0),
        &[("X-Fleet-Token", &t)],
        Some(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})),
    )
    .await;
    assert_eq!((status, body.as_str()), (202, ""));
    let (_, v) = rpc_at(
        srv.addr,
        &a.id,
        &t,
        &json!({"jsonrpc": "2.0", "id": 3, "method": "server/discover"}),
    )
    .await;
    assert_eq!(
        (v["id"].clone(), v["error"]["code"].clone()),
        (json!(3), json!(-32601))
    );
}

#[tokio::test]
async fn tools_list_names_both_tools() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    let t = token(&srv, &a.id).await;
    let (_, v) = rpc_at(
        srv.addr,
        &a.id,
        &t,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await;
    let tools = v["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("{v}"));
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["list_repos", "request_repo"]);
    assert_eq!(tools[1]["inputSchema"]["required"], json!(["repo", "why"]));
    let (_, v) = rpc_at(srv.addr, &a.id, &t, &call("list_repos", json!({}))).await;
    let (text, err) = result_of(&v);
    assert!(!err && text.contains("core"), "{text}");
}

#[tokio::test]
async fn request_for_a_repo_the_task_has_answers_at_once() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    let t = token(&srv, &a.id).await;
    let (_, v) = rpc_at(
        srv.addr,
        &a.id,
        &t,
        &call("request_repo", json!({"repo": "core", "why": "w"})),
    )
    .await;
    let (text, err) = result_of(&v);
    assert!(
        !err && text.starts_with(&format!("granted: core at {}", a.workspaces[0].worktree)),
        "{text}"
    );
    let now = agent(&srv.fx, &a.id).await;
    assert_eq!((now.repo_request, now.blocked_reason), (None, None));
}

#[tokio::test]
async fn request_puts_the_card_in_needs_you_with_repo_access() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    let t = token(&srv, &a.id).await;
    let handle = request_in_background(&srv, &a.id, &t, "tools");
    wait_pending(&srv.fx, &a.id).await;
    let now = agent(&srv.fx, &a.id).await;
    assert_eq!(
        now.repo_request,
        Some(RepoRequest {
            repo: "tools".into(),
            why: "need it".into()
        })
    );
    assert_eq!(
        (now.status, now.blocked_reason, now.activity.as_str()),
        (
            Status::NeedsYou,
            Some(BlockedReason::RepoAccess),
            "wants repo tools"
        )
    );
    assert!(
        !handle.is_finished(),
        "the call must be held until a decision"
    );
    handle.abort();
}

#[tokio::test]
async fn grant_attaches_and_answers_the_held_call() {
    let srv = start().await;
    let tools = tools_repo(&srv);
    let a = spawn(&srv, "x").await;
    let t = token(&srv, &a.id).await;
    let handle = request_in_background(&srv, &a.id, &t, tools.to_str().unwrap());
    wait_pending(&srv.fx, &a.id).await;
    let (status, v) = grant(&srv, &a.id, tools.to_str().unwrap()).await;
    assert_eq!(status, 200, "{v}");
    let (_, answer) = tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .unwrap()
        .unwrap();
    let (text, err) = result_of(&answer);
    let wt = PathBuf::from(&a.workdir).join("tools");
    assert!(!err && text.starts_with("granted:"), "{text}");
    assert_eq!(
        text,
        format!("granted: tools at {} (branch fleet/x)", wt.display())
    );
    assert!(wt.join("README").is_file());
    let now = agent(&srv.fx, &a.id).await;
    assert_eq!(
        (now.status, now.blocked_reason, now.repo_request),
        (Status::Working, None, None)
    );
    assert_eq!(now.workspaces.len(), 2);
}

#[tokio::test]
async fn grant_with_an_edited_target_attaches_the_edited_repo() {
    let srv = start().await;
    let tools = tools_repo(&srv);
    let other = make_repo(&srv.fx._tmp.path().join("other"));
    let a = spawn(&srv, "x").await;
    let t = token(&srv, &a.id).await;
    let handle = request_in_background(&srv, &a.id, &t, tools.to_str().unwrap());
    wait_pending(&srv.fx, &a.id).await;
    assert_eq!(grant(&srv, &a.id, other.to_str().unwrap()).await.0, 200);
    let (_, answer) = tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .unwrap()
        .unwrap();
    assert!(
        result_of(&answer).0.starts_with("granted: other at"),
        "{answer}"
    );
    let workdir = PathBuf::from(&a.workdir);
    assert!(workdir.join("other/README").is_file());
    assert!(!workdir.join("tools").exists());
}

#[tokio::test]
async fn refuse_answers_with_the_reason() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    let t = token(&srv, &a.id).await;
    for (reason, expected) in [
        ("not this one", "refused: not this one"),
        ("  ", "refused: no reason given"),
    ] {
        let handle = request_in_background(&srv, &a.id, &t, "tools");
        wait_pending(&srv.fx, &a.id).await;
        let (status, v) = refuse(&srv, &a.id, reason).await;
        assert_eq!(status, 204, "{v}");
        let (_, answer) = tokio::time::timeout(Duration::from_secs(10), handle)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result_of(&answer).0, expected);
        let now = agent(&srv.fx, &a.id).await;
        assert_eq!(
            (now.status, now.blocked_reason, now.repo_request),
            (Status::Working, None, None)
        );
        assert_eq!(now.workspaces.len(), 1);
    }
}

#[tokio::test]
async fn a_second_request_while_one_is_pending_is_an_error() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    let t = token(&srv, &a.id).await;
    let handle = request_in_background(&srv, &a.id, &t, "tools");
    wait_pending(&srv.fx, &a.id).await;
    let (_, v) = rpc_at(
        srv.addr,
        &a.id,
        &t,
        &call("request_repo", json!({"repo": "other", "why": "w"})),
    )
    .await;
    let (text, err) = result_of(&v);
    assert!(
        err && text.contains("a repo request is already waiting for an answer"),
        "{text}"
    );
    assert_eq!(
        agent(&srv.fx, &a.id).await.repo_request.unwrap().repo,
        "tools"
    );
    handle.abort();
}

#[tokio::test]
async fn grant_or_refuse_with_no_pending_request_is_a_conflict() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    assert_eq!(grant(&srv, &a.id, "core").await.0, 409);
    assert_eq!(refuse(&srv, &a.id, "no").await.0, 409);
    assert_eq!(grant(&srv, &AgentId("zz".into()), "core").await.0, 404);
}

#[tokio::test]
async fn hooks_do_not_move_a_card_off_a_pending_request() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    let t = token(&srv, &a.id).await;
    let handle = request_in_background(&srv, &a.id, &t, "tools");
    wait_pending(&srv.fx, &a.id).await;
    let now = srv.fx.clock.now_for_test();
    let pre = json!({"hook_event_name": "PreToolUse", "tool_name": "mcp__fleet__request_repo"});
    ingest(&srv.fx.store, &a.id, Some(&t), pre, now)
        .await
        .unwrap();
    let mid = agent(&srv.fx, &a.id).await;
    assert_eq!(
        (mid.status, mid.blocked_reason),
        (Status::NeedsYou, Some(BlockedReason::RepoAccess))
    );
    assert_eq!(
        mid.activity, "running mcp__fleet__request_repo",
        "activity may still move"
    );
    ingest(
        &srv.fx.store,
        &a.id,
        Some(&t),
        json!({"hook_event_name": "Stop"}),
        now,
    )
    .await
    .unwrap();
    let after = agent(&srv.fx, &a.id).await;
    assert_eq!(
        (after.status, after.blocked_reason),
        (Status::NeedsYou, Some(BlockedReason::RepoAccess))
    );
    assert!(after.repo_request.is_some());
    handle.abort();
}

#[tokio::test]
async fn startup_clears_an_interrupted_request() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    srv.fx
        .store
        .update_agent(&a.id, |x| {
            x.status = Status::NeedsYou;
            x.blocked_reason = Some(BlockedReason::RepoAccess);
            x.activity = "wants repo tools".into();
            x.repo_request = Some(RepoRequest {
                repo: "tools".into(),
                why: "w".into(),
            });
        })
        .await
        .unwrap();
    srv.fx.svc.clear_interrupted_requests().await.unwrap();
    let now = agent(&srv.fx, &a.id).await;
    assert_eq!(
        (now.status, now.blocked_reason, now.repo_request),
        (Status::Working, None, None)
    );
    assert_eq!(now.activity, "repo request interrupted by daemon restart");
}

#[tokio::test]
async fn spawn_passes_mcp_config_and_the_file_is_private() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    let t = token(&srv, &a.id).await;
    let path = srv
        .fx
        .state_dir
        .join("agents")
        .join(&a.id.0)
        .join("mcp.json");
    assert!(pane_contains(&srv.fx, &a.id, "arg=--mcp-config").await);
    assert!(pane_contains(&srv.fx, &a.id, &format!("arg={}", path.display())).await);
    // Owner-only is a Unix file mode; Windows has no such bits.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let fleet = &v["mcpServers"]["fleet"];
    assert_eq!(fleet["type"], "http");
    assert_eq!(
        fleet["url"],
        format!("http://127.0.0.1:7878/internal/mcp/{}", a.id.0)
    );
    assert_eq!(fleet["headers"]["X-Fleet-Token"], t);
    // Without these Claude Code abandons a held call after 60 s (see the MCP spike).
    for env in [
        "mcp_timeout=2400000",
        "mcp_idle=0",
        "mcp_background=3600000",
    ] {
        assert!(
            pane_contains(&srv.fx, &a.id, env).await,
            "{env} not in the agent's environment"
        );
    }
}

#[tokio::test]
async fn a_decision_after_the_held_call_is_gone_is_typed_into_the_session() {
    let srv = start().await;
    let tools = tools_repo(&srv);
    let a = spawn(&srv, "x").await;
    let t = token(&srv, &a.id).await;
    let handle = request_in_background(&srv, &a.id, &t, tools.to_str().unwrap());
    wait_pending(&srv.fx, &a.id).await;
    assert!(wait_until(|| async { srv.fx.svc.is_holding(&a.id) }).await);
    handle.abort();
    let _ = handle.await;
    // Until the server drops the handler a decision would still go to the (dead) call instead of being typed.
    assert!(wait_until(|| async { !srv.fx.svc.is_holding(&a.id) }).await);
    let (status, v) = grant(&srv, &a.id, tools.to_str().unwrap()).await;
    assert_eq!(status, 200, "{v}");
    let wt = PathBuf::from(&a.workdir).join("tools");
    assert!(
        pane_contains(
            &srv.fx,
            &a.id,
            &format!(
                "Fleet Manager: granted tools at {} (branch fleet/x)",
                wt.display()
            )
        )
        .await
    );
    assert!(wt.join("README").is_file());
    let now = agent(&srv.fx, &a.id).await;
    assert_eq!((now.status, now.repo_request), (Status::Working, None));

    let handle = request_in_background(&srv, &a.id, &t, "elsewhere");
    wait_pending(&srv.fx, &a.id).await;
    assert!(wait_until(|| async { srv.fx.svc.is_holding(&a.id) }).await);
    handle.abort();
    let _ = handle.await;
    assert!(wait_until(|| async { !srv.fx.svc.is_holding(&a.id) }).await);
    assert_eq!(refuse(&srv, &a.id, "no").await.0, 204);
    assert!(pane_contains(&srv.fx, &a.id, "Fleet Manager: refused elsewhere: no").await);
}

#[tokio::test]
async fn a_hold_past_its_limit_ends_the_call_and_the_decision_is_typed() {
    let fx = fixture().await;
    let mut opts = fx.svc.options().clone();
    opts.repo_request_hold = Duration::from_millis(300);
    let svc = Arc::new(AgentService::new(
        fx.svc.config().clone(),
        fx.store.clone(),
        fx.repos.clone(),
        fx.sessions.clone(),
        opts,
    ));
    let srv = serve_with(fx, svc.clone()).await;
    let req = SpawnRequest {
        actor: Actor("local".into()),
        product: PRODUCT.into(),
        release: None,
        repos: vec![pick(&srv.fx, "core")],
        prompt: "x".into(),
        slug: None,
        persona: PERSONA.into(),
        kind: fleet_api::AgentKind::Solo,
        role: None,
    };
    let a = svc.spawn(req).await.unwrap();
    assert!(pane_contains(&srv.fx, &a.id, "ready").await);
    let t = token(&srv, &a.id).await;
    let (_, v) = rpc_at(
        srv.addr,
        &a.id,
        &t,
        &call("request_repo", json!({"repo": "elsewhere", "why": "w"})),
    )
    .await;
    let (text, err) = result_of(&v);
    assert!(err && text.contains("typed into this session"), "{text}");
    assert!(
        agent(&srv.fx, &a.id).await.repo_request.is_some(),
        "the request stays on the card"
    );
    svc.refuse_repo(&a.id, "later").await.unwrap();
    assert!(pane_contains(&srv.fx, &a.id, "Fleet Manager: refused elsewhere: later").await);
}

#[tokio::test]
async fn stop_ends_a_held_call() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    let t = token(&srv, &a.id).await;
    let handle = request_in_background(&srv, &a.id, &t, "tools");
    wait_pending(&srv.fx, &a.id).await;
    srv.fx.svc.stop(&a.id).await.unwrap();
    let (_, v) = tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .unwrap()
        .unwrap();
    assert!(result_of(&v).1, "{v}");
}

/// A request whose write lands before Stop's Stopped write, but whose waiter is registered after Stop removed the
/// waiters, must still end with the stop instead of being held for the whole hold.
#[tokio::test(flavor = "multi_thread")]
async fn a_request_racing_a_stop_is_not_held() {
    let fx = fixture().await;
    let gate = Arc::new(RequestGate::default());
    let mut opts = fx.svc.options().clone();
    opts.request_gate = Some(gate.clone());
    opts.repo_request_hold = Duration::from_secs(60);
    let svc = Arc::new(AgentService::new(
        fx.svc.config().clone(),
        fx.store.clone(),
        fx.repos.clone(),
        fx.sessions.clone(),
        opts,
    ));
    let req = SpawnRequest {
        actor: Actor("local".into()),
        product: PRODUCT.into(),
        release: None,
        repos: vec![pick(&fx, "core")],
        prompt: "x".into(),
        slug: None,
        persona: PERSONA.into(),
        kind: fleet_api::AgentKind::Solo,
        role: None,
    };
    let a = svc.spawn(req).await.unwrap();
    assert!(pane_contains(&fx, &a.id, "ready").await);
    let (svc2, id) = (svc.clone(), a.id.clone());
    let handle = tokio::spawn(async move { svc2.request_repo(&id, "elsewhere", "w").await });
    tokio::time::timeout(Duration::from_secs(10), gate.reached.notified())
        .await
        .expect("the request never reached the gap");
    svc.stop(&a.id).await.unwrap();
    gate.open.notify_one();
    let r = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("the request was held after the stop")
        .unwrap();
    assert!(matches!(r, Err(ServiceError::NotRunning)), "{r:?}");
    assert!(!svc.is_holding(&a.id));
}

#[tokio::test]
async fn a_stopped_agent_is_refused_mcp_as_an_unknown_one() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    let t = token(&srv, &a.id).await;
    srv.fx.svc.stop(&a.id).await.unwrap();
    let body = call("request_repo", json!({"repo": "tools", "why": "need it"}));
    let (status, v) =
        tokio::time::timeout(Duration::from_secs(10), rpc_at(srv.addr, &a.id, &t, &body))
            .await
            .expect("the call was held instead of refused");
    assert_eq!(status, 404, "{v}");
    // The service refuses on its own too, whoever reaches it.
    let r = tokio::time::timeout(
        Duration::from_secs(5),
        srv.fx.svc.request_repo(&a.id, "tools", "need it"),
    )
    .await
    .expect("the service held a stopped agent's request");
    assert!(matches!(r, Err(ServiceError::NotRunning)), "{r:?}");
    let after = agent(&srv.fx, &a.id).await;
    assert_eq!(
        (after.status, after.stop_cause, after.repo_request),
        (Status::Stopped, Some(StopCause::Stopped), None)
    );
    let p = srv.fx.store.task(&a.id).await.unwrap().unwrap();
    assert_eq!(
        (p.state, p.stop_cause),
        (TaskState::Stopped, Some(StopCause::Stopped))
    );
}

/// A client that is already connected only learns of a request through the event stream, so the status event
/// carries it while it is pending and clears it when it is decided.
#[tokio::test]
async fn the_status_event_carries_the_request_and_its_end() {
    use fleet_api::EventKind;
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    let t = token(&srv, &a.id).await;
    let mut rx = srv.fx.store.subscribe();
    let handle = request_in_background(&srv, &a.id, &t, "tools");
    wait_pending(&srv.fx, &a.id).await;
    assert_eq!(refuse(&srv, &a.id, "no").await.0, 204);
    let _ = tokio::time::timeout(Duration::from_secs(10), handle).await;
    let mut seen = vec![];
    while let Ok(e) = rx.try_recv() {
        if let EventKind::StatusChanged {
            id, repo_request, ..
        } = e.kind
        {
            if id == a.id {
                seen.push(repo_request);
            }
        }
    }
    assert_eq!(
        seen.first(),
        Some(&Some(RepoRequest {
            repo: "tools".into(),
            why: "need it".into()
        })),
        "{seen:?}"
    );
    assert_eq!(seen.last(), Some(&None), "{seen:?}");
}

/// The fallback types the decision only when the card was waiting on the request itself, read before the decision
/// moves it to Working: anything else may be a dialog that Enter would answer.
#[tokio::test]
async fn the_fallback_types_only_onto_the_request_it_answers() {
    let fx = fixture().await;
    let mut opts = fx.svc.options().clone();
    opts.repo_request_hold = Duration::from_millis(100);
    let svc = Arc::new(AgentService::new(
        fx.svc.config().clone(),
        fx.store.clone(),
        fx.repos.clone(),
        fx.sessions.clone(),
        opts,
    ));
    let req = SpawnRequest {
        actor: Actor("local".into()),
        product: PRODUCT.into(),
        release: None,
        repos: vec![pick(&fx, "core")],
        prompt: "x".into(),
        slug: None,
        persona: PERSONA.into(),
        kind: fleet_api::AgentKind::Solo,
        role: None,
    };
    let a = svc.spawn(req).await.unwrap();
    assert!(pane_contains(&fx, &a.id, "ready").await);
    let tools = make_repo(&fx._tmp.path().join("tools"));
    assert!(
        svc.request_repo(&a.id, tools.to_str().unwrap(), "w")
            .await
            .is_err(),
        "the hold runs out"
    );
    svc.grant_repo(&a.id, tools.to_str().unwrap())
        .await
        .unwrap();
    assert!(pane_contains(&fx, &a.id, "Fleet Manager: granted tools at").await);

    assert!(svc.request_repo(&a.id, "elsewhere", "w").await.is_err());
    // A permission prompt came up meanwhile (as the hooks would record it).
    fx.store
        .update_agent(&a.id, |x| {
            x.blocked_reason = Some(BlockedReason::Permission)
        })
        .await
        .unwrap();
    svc.refuse_repo(&a.id, "no").await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let pane = fx.sessions.screen_text(&session_name(&a.id)).unwrap();
    assert!(!pane.contains("refused elsewhere"), "{pane}");
}

async fn tool_names(srv: &Server, id: &AgentId) -> Vec<String> {
    let t = token(srv, id).await;
    let (_, v) = rpc_at(
        srv.addr,
        id,
        &t,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await;
    v["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("{v}"))
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn propose_breakdown_schema_offers_mode_and_effort() {
    let srv = start().await;
    add_origin_with_workflow(&srv.fx, &one_step("true"));
    let lead = srv
        .fx
        .svc
        .spawn(SpawnRequest {
            actor: Actor("local".into()),
            product: PRODUCT.into(),
            release: None,
            repos: vec![pick(&srv.fx, "core")],
            prompt: "lead".into(),
            slug: None,
            persona: PERSONA.into(),
            kind: fleet_api::AgentKind::Orchestrator,
            role: Some("direct".into()),
        })
        .await
        .unwrap();
    let t = token(&srv, &lead.id).await;
    let (_, v) = rpc_at(
        srv.addr,
        &lead.id,
        &t,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await;
    let tool = v["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "propose_breakdown")
        .unwrap();
    let props = &tool["inputSchema"]["properties"]["steps"]["items"]["properties"];
    assert_eq!(
        props["mode"]["enum"],
        json!(["auto", "acceptEdits", "manual"])
    );
    assert_eq!(
        props["effort"]["enum"],
        json!(["low", "medium", "high", "xhigh", "max"])
    );
    assert!(props["mode"]["description"]
        .as_str()
        .unwrap()
        .contains("stricter"));
}

async fn call_as(srv: &Server, id: &AgentId, name: &str, args: Value) -> (String, bool) {
    let t = token(srv, id).await;
    let (_, v) = rpc_at(srv.addr, id, &t, &call(name, args)).await;
    result_of(&v)
}

#[tokio::test]
async fn tools_by_kind() {
    let srv = start().await;
    let solo = spawn(&srv, "x").await;
    assert_eq!(
        tool_names(&srv, &solo.id).await,
        ["list_repos", "request_repo"]
    );
    let (text, err) = call_as(&srv, &solo.id, "spawn_specialist", json!({"step": 1})).await;
    assert_eq!(
        (text.as_str(), err),
        ("spawn_specialist is not available to this agent", true)
    );

    add_origin_with_workflow(&srv.fx, &one_step("true"));
    let lead = srv
        .fx
        .svc
        .spawn(SpawnRequest {
            actor: Actor("local".into()),
            product: PRODUCT.into(),
            release: None,
            repos: vec![pick(&srv.fx, "core")],
            prompt: "lead".into(),
            slug: None,
            persona: PERSONA.into(),
            kind: fleet_api::AgentKind::Orchestrator,
            role: Some("direct".into()),
        })
        .await
        .unwrap();
    assert_eq!(
        tool_names(&srv, &lead.id).await,
        [
            "list_repos",
            "propose_breakdown",
            "spawn_specialist",
            "wait_for_fleet",
            "fleet_status",
            "open_pr"
        ]
    );
    let (text, err) = call_as(&srv, &lead.id, "finish_step", json!({"report": "done"})).await;
    assert_eq!(
        (text.as_str(), err),
        ("finish_step is not available to this agent", true)
    );
    let (text, err) = call_as(
        &srv,
        &lead.id,
        "request_repo",
        json!({"repo": "other", "why": "need it"}),
    )
    .await;
    assert_eq!(
        (text.as_str(), err),
        ("an orchestrated task has one repo", true)
    );
    assert_eq!(agent(&srv.fx, &lead.id).await.repo_request, None);
    let (text, err) = call_as(&srv, &lead.id, "fleet_status", json!({})).await;
    assert_eq!((text.as_str(), err), ("no breakdown yet", true));

    // A fleet agent of the lead's task, stored as the specialist spawn will store it.
    let mut sp = lead.clone();
    sp.id = AgentId("specialist1".into());
    sp.session = "fleet-specialist".into();
    sp.kind = fleet_api::AgentKind::Specialist;
    sp.task = Some(lead.id.clone());
    sp.role = Some("rust".into());
    sp.step = Some(1);
    srv.fx.store.insert_agent(&sp, "sptoken").await.unwrap();
    assert_eq!(
        tool_names(&srv, &sp.id).await,
        ["list_repos", "finish_step"]
    );
    let (text, err) = call_as(
        &srv,
        &sp.id,
        "request_repo",
        json!({"repo": "other", "why": "need it"}),
    )
    .await;
    assert_eq!(
        (text.as_str(), err),
        ("this task has one repo; ask the orchestrator", true)
    );
    let (text, err) = call_as(&srv, &sp.id, "open_pr", json!({"title": "t", "body": "b"})).await;
    assert_eq!(
        (text.as_str(), err),
        ("open_pr is not available to this agent", true)
    );
    let (text, err) = call_as(&srv, &sp.id, "list_repos", json!({})).await;
    assert!(!err && text.contains("core"), "{text}");
}

async fn spawn_lead(srv: &Server) -> Agent {
    add_origin_with_workflow(&srv.fx, &one_step("true"));
    srv.fx
        .svc
        .spawn(SpawnRequest {
            actor: Actor("local".into()),
            product: PRODUCT.into(),
            release: None,
            repos: vec![pick(&srv.fx, "core")],
            prompt: "lead".into(),
            slug: None,
            persona: PERSONA.into(),
            kind: fleet_api::AgentKind::Orchestrator,
            role: Some("direct".into()),
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn propose_breakdown_over_mcp() {
    let srv = start().await;
    let lead = spawn_lead(&srv).await;
    let before = agent(&srv.fx, &lead.id).await;
    for (args, wanted) in [
        (json!({"steps": []}), "the breakdown has no steps"),
        (json!({}), "propose_breakdown needs `steps`"),
        (
            json!({"steps": [{"title": "api", "specialist": "go", "brief": "b"}]}),
            "step 1",
        ),
    ] {
        let (text, err) = call_as(&srv, &lead.id, "propose_breakdown", args).await;
        assert!(err && text.contains(wanted), "{text}");
    }
    let after = agent(&srv.fx, &lead.id).await;
    assert_eq!(
        (after.status, after.blocked_reason),
        (before.status, before.blocked_reason)
    );

    let t = token(&srv, &lead.id).await;
    let body = call(
        "propose_breakdown",
        json!({"steps": [{"title": "api", "specialist": "rust", "brief": "build it"}]}),
    );
    let (addr, id) = (srv.addr, lead.id.clone());
    let handle = tokio::spawn(async move { rpc_at(addr, &id, &t, &body).await });
    assert!(
        wait_until(|| async {
            agent(&srv.fx, &lead.id).await.blocked_reason == Some(BlockedReason::Breakdown)
        })
        .await
    );
    // The hooks around the held call do not take the card off the proposal.
    let t = token(&srv, &lead.id).await;
    let now = srv.fx.clock.now_for_test();
    for hook in [
        json!({"hook_event_name": "PreToolUse", "tool_name": "mcp__fleet__propose_breakdown"}),
        json!({"hook_event_name": "Stop"}),
    ] {
        ingest(&srv.fx.store, &lead.id, Some(&t), hook, now)
            .await
            .unwrap();
    }
    let mid = agent(&srv.fx, &lead.id).await;
    assert_eq!(
        (mid.status, mid.blocked_reason),
        (Status::NeedsYou, Some(BlockedReason::Breakdown))
    );
    let (status, _) = json_of(
        srv.addr,
        "POST",
        &format!("/api/tasks/{}/breakdown/decide", lead.id.0),
        Some(&json!({"approve": false, "comment": "smaller"})),
    )
    .await;
    assert_eq!(status, 204);
    let (_, v) = tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        result_of(&v),
        ("changes requested: smaller".to_string(), false)
    );
}

#[tokio::test]
async fn spawn_and_finish_over_mcp() {
    let srv = start().await;
    let lead = spawn_lead(&srv).await;
    let (text, err) = call_as(&srv, &lead.id, "spawn_specialist", json!({"step": 1})).await;
    assert_eq!((text.as_str(), err), ("no breakdown is approved yet", true));
    let (text, err) = call_as(&srv, &lead.id, "spawn_specialist", json!({})).await;
    assert!(err && text.contains("needs `step`"), "{text}");

    let svc = srv.fx.svc.clone();
    let id = lead.id.clone();
    let held = tokio::spawn(async move {
        svc.propose_breakdown(
            &id,
            vec![fleet_api::StepDraft {
                n: None,
                title: "api".into(),
                specialist: "rust".into(),
                brief: "build it".into(),
                depends_on: vec![],
                release_note: false,
                mode: None,
                effort: None,
            }],
        )
        .await
    });
    assert!(
        wait_until(|| async {
            agent(&srv.fx, &lead.id).await.blocked_reason == Some(BlockedReason::Breakdown)
        })
        .await
    );
    srv.fx
        .svc
        .decide_breakdown(
            &lead.id,
            fleet_api::DecideBreakdown {
                approve: true,
                comment: None,
                version: None,
            },
        )
        .await
        .unwrap();
    held.await.unwrap().unwrap();

    let (text, err) = call_as(&srv, &lead.id, "spawn_specialist", json!({"step": 1})).await;
    assert!(!err, "{text}");
    let sp = srv
        .fx
        .store
        .snapshot()
        .await
        .unwrap()
        .agents
        .into_iter()
        .find(|a| a.kind == fleet_api::AgentKind::Specialist)
        .unwrap();
    let branch = &sp.workspaces[0].branch;
    assert_eq!(
        text,
        format!("step 1 started: agent {}, branch {branch}", sp.id.0)
    );

    let (text, err) = call_as(&srv, &lead.id, "open_pr", json!({"title": "t"})).await;
    assert!(err && text.contains("needs `title` and `body`"), "{text}");
    let (text, err) = call_as(
        &srv,
        &lead.id,
        "open_pr",
        json!({"title": "t", "body": "b"}),
    )
    .await;
    assert_eq!((text.as_str(), err), ("step 1 is running", true));

    let wt = PathBuf::from(&sp.workspaces[0].worktree);
    std::fs::write(wt.join("api.rs"), "").unwrap();
    let (text, err) = call_as(&srv, &sp.id, "finish_step", json!({"report": "built"})).await;
    assert_eq!((text.as_str(), err), ("commit your changes first", true));
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-q", "-m", "api"]);
    let (text, err) = call_as(&srv, &sp.id, "finish_step", json!({"report": "built"})).await;
    assert_eq!((text.as_str(), err), ("step 1 done", false));
    assert_eq!(agent(&srv.fx, &sp.id).await.status, Status::Stopped);

    // A failed gate tells the orchestrator what the tests said, so it can propose a fix.
    push_workflow(&srv.fx, &one_step("echo boom; exit 3"));
    srv.fx
        .store
        .update_task(&lead.id, |t| t.pr_rules = None)
        .await
        .unwrap();
    let lead_wt = PathBuf::from(&lead.workspaces[0].worktree);
    std::fs::write(lead_wt.join("api.rs"), "").unwrap();
    git(&lead_wt, &["add", "."]);
    git(&lead_wt, &["commit", "-q", "-m", "merged"]);
    let (text, err) = call_as(
        &srv,
        &lead.id,
        "open_pr",
        json!({"title": "t", "body": "b"}),
    )
    .await;
    assert_eq!(
        (text.as_str(), err),
        ("tests failed in step test (exit 3)\nboom", true)
    );
}

#[tokio::test]
async fn request_repo_resolves_within_the_product() {
    let srv = start().await;
    let tmp = srv.fx._tmp.path();
    // Two products each have a repo `lib`; only this task's product's may be attached.
    let ours = make_repo(&tmp.join("ours/lib"));
    std::fs::write(ours.join("OURS"), "x").unwrap();
    git(&ours, &["add", "."]);
    git(&ours, &["commit", "-q", "-m", "ours"]);
    let theirs = make_repo(&tmp.join("theirs/lib"));
    srv.fx
        .store
        .insert_product(&fleet_daemon::store::ProductRow {
            id: "q".into(),
            name: "q".into(),
            default_persona: None,
            internal_url: None,
            created_at: 2,
        })
        .await
        .unwrap();
    for (product, path) in [(PRODUCT, &ours), ("q", &theirs)] {
        srv.fx
            .store
            .upsert_repo(&fleet_daemon::store::RepoRow {
                product: product.into(),
                name: "lib".into(),
                path: Some(path.clone()),
                url: None,
            })
            .await
            .unwrap();
    }
    let a = spawn(&srv, "x").await;
    assert_eq!(a.product, PRODUCT);
    let (listing, err) = call_as(&srv, &a.id, "list_repos", json!({})).await;
    assert!(!err, "{listing}");
    let listed: Value = serde_json::from_str(&listing).unwrap();
    assert_eq!(
        listed["remembered"],
        json!([{"name": "core", "source": "path"}, {"name": "lib", "source": "path"}])
    );
    let t = token(&srv, &a.id).await;
    let handle = request_in_background(&srv, &a.id, &t, "lib");
    wait_pending(&srv.fx, &a.id).await;
    let (status, v) = grant(&srv, &a.id, "lib").await;
    assert_eq!(status, 200, "{v}");
    let (_, answer) = tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .unwrap()
        .unwrap();
    assert!(
        result_of(&answer).0.starts_with("granted: lib at"),
        "{answer}"
    );
    let wt = PathBuf::from(&a.workdir).join("lib");
    assert!(
        wt.join("OURS").is_file(),
        "attached the other product's lib"
    );
    assert_eq!(
        git(&wt, &["rev-parse", "HEAD"]),
        git(&ours, &["rev-parse", "HEAD"])
    );
    assert!(git(&theirs, &["branch", "--list", "fleet/*"]).is_empty());
}
