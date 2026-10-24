mod common;

use common::net::*;
use common::*;
use fleet_api::{Actor, Agent, AgentId, BlockedReason, RepoRequest, SpawnRequest, Status};
use fleet_daemon::agents::AgentService;
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
    let state = AppState::new(svc, fx.store.clone(), fx.tmux.clone(), addr.port());
    tokio::spawn(async move { axum::serve(listener, router(state)).await });
    Server { addr, fx }
}

async fn start() -> Server {
    let fx = fixture().await;
    let svc = fx.svc.clone();
    serve_with(fx, svc).await
}

async fn spawn(srv: &Server, prompt: &str) -> Agent {
    let req = SpawnRequest { actor: Actor("local".into()), repos: vec!["core".into()], prompt: prompt.into(), task_slug: None };
    let a = srv.fx.svc.spawn(req).await.unwrap();
    assert!(pane_contains(&srv.fx, &a.id, "ready").await);
    a
}

async fn token(srv: &Server, id: &AgentId) -> String {
    srv.fx.store.agent_token(id).await.unwrap().unwrap()
}

/// One JSON-RPC message to the agent's MCP endpoint; returns the HTTP status and the parsed body.
async fn rpc_at(addr: SocketAddr, id: &AgentId, token: &str, body: &Value) -> (u16, Value) {
    let (status, text) = http(addr, "POST", &format!("/internal/mcp/{}", id.0), &[("X-Fleet-Token", token)], Some(body)).await;
    (status, serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

fn call(name: &str, args: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": name, "arguments": args}})
}

/// The tool result's text and isError flag.
fn result_of(v: &Value) -> (String, bool) {
    let r = &v["result"];
    (r["content"][0]["text"].as_str().unwrap_or_else(|| panic!("no text in {v}")).to_string(), r["isError"].as_bool().unwrap())
}

/// Starts a held `request_repo` call in the background.
fn request_in_background(srv: &Server, id: &AgentId, token: &str, repo: &str) -> tokio::task::JoinHandle<(u16, Value)> {
    let (addr, id, token, body) = (srv.addr, id.clone(), token.to_string(), call("request_repo", json!({"repo": repo, "why": "need it"})));
    tokio::spawn(async move { rpc_at(addr, &id, &token, &body).await })
}

async fn wait_pending(fx: &Fx, id: &AgentId) {
    assert!(wait_until(|| async { agent(fx, id).await.repo_request.is_some() }).await, "request never became pending");
}

async fn grant(srv: &Server, id: &AgentId, repo: &str) -> (u16, Value) {
    json_of(srv.addr, "POST", &format!("/api/agents/{}/repo-request/grant", id.0), Some(&json!({"actor": "local", "repo": repo}))).await
}

async fn refuse(srv: &Server, id: &AgentId, reason: &str) -> (u16, Value) {
    json_of(srv.addr, "POST", &format!("/api/agents/{}/repo-request/refuse", id.0), Some(&json!({"actor": "local", "reason": reason}))).await
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
    assert_eq!(http(srv.addr, "POST", &path, &[("X-Fleet-Token", "wrong")], Some(&body)).await.0, 401);
    assert_eq!(http(srv.addr, "POST", &path, &[], Some(&body)).await.0, 401);
    let t = token(&srv, &a.id).await;
    assert_eq!(http(srv.addr, "POST", "/internal/mcp/nobody", &[("X-Fleet-Token", &t)], Some(&body)).await.0, 404);
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
    assert_eq!((v["id"].clone(), v["result"]["protocolVersion"].clone()), (json!(0), json!("2025-11-25")));
    assert_eq!(v["result"]["capabilities"], json!({"tools": {}}));
    assert_eq!(v["result"]["serverInfo"]["name"], "fleet-board");
    let (status, body) = http(srv.addr, "POST", &format!("/internal/mcp/{}", a.id.0), &[("X-Fleet-Token", &t)], Some(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))).await;
    assert_eq!((status, body.as_str()), (202, ""));
    let (_, v) = rpc_at(srv.addr, &a.id, &t, &json!({"jsonrpc": "2.0", "id": 3, "method": "server/discover"})).await;
    assert_eq!((v["id"].clone(), v["error"]["code"].clone()), (json!(3), json!(-32601)));
}

#[tokio::test]
async fn tools_list_names_both_tools() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    let t = token(&srv, &a.id).await;
    let (_, v) = rpc_at(srv.addr, &a.id, &t, &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).await;
    let tools = v["result"]["tools"].as_array().unwrap_or_else(|| panic!("{v}"));
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["list_repos", "request_repo"]);
    assert_eq!(tools[1]["inputSchema"]["required"], json!(["repo", "why"]));
    let (_, v) = rpc_at(srv.addr, &a.id, &t, &call("list_repos", json!({}))).await;
    let (text, err) = result_of(&v);
    assert!(!err && text.contains("core"), "{text}");
}

#[tokio::test]
async fn request_for_a_repo_the_project_has_answers_at_once() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    let t = token(&srv, &a.id).await;
    let (_, v) = rpc_at(srv.addr, &a.id, &t, &call("request_repo", json!({"repo": "core", "why": "w"}))).await;
    let (text, err) = result_of(&v);
    assert!(!err && text.starts_with(&format!("granted: core at {}", a.workspaces[0].worktree)), "{text}");
    let now = agent(&srv.fx, &a.id).await;
    assert_eq!((now.repo_request, now.blocked_reason), (None, None));
}

#[tokio::test]
async fn request_puts_the_card_in_needs_you_with_repo_access() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    let t = token(&srv, &a.id).await;
    let task = request_in_background(&srv, &a.id, &t, "tools");
    wait_pending(&srv.fx, &a.id).await;
    let now = agent(&srv.fx, &a.id).await;
    assert_eq!(now.repo_request, Some(RepoRequest { repo: "tools".into(), why: "need it".into() }));
    assert_eq!((now.status, now.blocked_reason, now.activity.as_str()), (Status::NeedsYou, Some(BlockedReason::RepoAccess), "wants repo tools"));
    assert!(!task.is_finished(), "the call must be held until a decision");
    task.abort();
}

#[tokio::test]
async fn grant_attaches_and_answers_the_held_call() {
    let srv = start().await;
    let tools = tools_repo(&srv);
    let a = spawn(&srv, "x").await;
    let t = token(&srv, &a.id).await;
    let task = request_in_background(&srv, &a.id, &t, tools.to_str().unwrap());
    wait_pending(&srv.fx, &a.id).await;
    let (status, v) = grant(&srv, &a.id, tools.to_str().unwrap()).await;
    assert_eq!(status, 200, "{v}");
    let (_, answer) = tokio::time::timeout(Duration::from_secs(10), task).await.unwrap().unwrap();
    let (text, err) = result_of(&answer);
    let wt = PathBuf::from(&a.workdir).join("tools");
    assert!(!err && text.starts_with("granted:"), "{text}");
    assert_eq!(text, format!("granted: tools at {} (branch fleet/x)", wt.display()));
    assert!(wt.join("README").is_file());
    let now = agent(&srv.fx, &a.id).await;
    assert_eq!((now.status, now.blocked_reason, now.repo_request), (Status::Working, None, None));
    assert_eq!(now.workspaces.len(), 2);
}

#[tokio::test]
async fn grant_with_an_edited_target_attaches_the_edited_repo() {
    let srv = start().await;
    let tools = tools_repo(&srv);
    let other = make_repo(&srv.fx._tmp.path().join("other"));
    let a = spawn(&srv, "x").await;
    let t = token(&srv, &a.id).await;
    let task = request_in_background(&srv, &a.id, &t, tools.to_str().unwrap());
    wait_pending(&srv.fx, &a.id).await;
    assert_eq!(grant(&srv, &a.id, other.to_str().unwrap()).await.0, 200);
    let (_, answer) = tokio::time::timeout(Duration::from_secs(10), task).await.unwrap().unwrap();
    assert!(result_of(&answer).0.starts_with("granted: other at"), "{answer}");
    let workdir = PathBuf::from(&a.workdir);
    assert!(workdir.join("other/README").is_file());
    assert!(!workdir.join("tools").exists());
}

#[tokio::test]
async fn refuse_answers_with_the_reason() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    let t = token(&srv, &a.id).await;
    for (reason, expected) in [("not this one", "refused: not this one"), ("  ", "refused: no reason given")] {
        let task = request_in_background(&srv, &a.id, &t, "tools");
        wait_pending(&srv.fx, &a.id).await;
        let (status, v) = refuse(&srv, &a.id, reason).await;
        assert_eq!(status, 204, "{v}");
        let (_, answer) = tokio::time::timeout(Duration::from_secs(10), task).await.unwrap().unwrap();
        assert_eq!(result_of(&answer).0, expected);
        let now = agent(&srv.fx, &a.id).await;
        assert_eq!((now.status, now.blocked_reason, now.repo_request), (Status::Working, None, None));
        assert_eq!(now.workspaces.len(), 1);
    }
}

#[tokio::test]
async fn a_second_request_while_one_is_pending_is_an_error() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    let t = token(&srv, &a.id).await;
    let task = request_in_background(&srv, &a.id, &t, "tools");
    wait_pending(&srv.fx, &a.id).await;
    let (_, v) = rpc_at(srv.addr, &a.id, &t, &call("request_repo", json!({"repo": "other", "why": "w"}))).await;
    let (text, err) = result_of(&v);
    assert!(err && text.contains("a repo request is already waiting for an answer"), "{text}");
    assert_eq!(agent(&srv.fx, &a.id).await.repo_request.unwrap().repo, "tools");
    task.abort();
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
    let task = request_in_background(&srv, &a.id, &t, "tools");
    wait_pending(&srv.fx, &a.id).await;
    let now = srv.fx.clock.now_for_test();
    let pre = json!({"hook_event_name": "PreToolUse", "tool_name": "mcp__fleet__request_repo"});
    ingest(&srv.fx.store, &a.id, Some(&t), pre, now).await.unwrap();
    let mid = agent(&srv.fx, &a.id).await;
    assert_eq!((mid.status, mid.blocked_reason), (Status::NeedsYou, Some(BlockedReason::RepoAccess)));
    assert_eq!(mid.activity, "running mcp__fleet__request_repo", "activity may still move");
    ingest(&srv.fx.store, &a.id, Some(&t), json!({"hook_event_name": "Stop"}), now).await.unwrap();
    let after = agent(&srv.fx, &a.id).await;
    assert_eq!((after.status, after.blocked_reason), (Status::NeedsYou, Some(BlockedReason::RepoAccess)));
    assert!(after.repo_request.is_some());
    task.abort();
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
            x.repo_request = Some(RepoRequest { repo: "tools".into(), why: "w".into() });
        })
        .await
        .unwrap();
    srv.fx.svc.clear_interrupted_requests().await.unwrap();
    let now = agent(&srv.fx, &a.id).await;
    assert_eq!((now.status, now.blocked_reason, now.repo_request), (Status::Working, None, None));
    assert_eq!(now.activity, "repo request interrupted by daemon restart");
}

#[tokio::test]
async fn spawn_passes_mcp_config_and_the_file_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    let t = token(&srv, &a.id).await;
    let path = srv.fx.state_dir.join("agents").join(&a.id.0).join("mcp.json");
    assert!(pane_contains(&srv.fx, &a.id, "arg=--mcp-config").await);
    assert!(pane_contains(&srv.fx, &a.id, &format!("arg={}", path.display())).await);
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let fleet = &v["mcpServers"]["fleet"];
    assert_eq!(fleet["type"], "http");
    assert_eq!(fleet["url"], format!("http://127.0.0.1:7878/internal/mcp/{}", a.id.0));
    assert_eq!(fleet["headers"]["X-Fleet-Token"], t);
    // Without these Claude Code abandons a held call after 60 s (see the MCP spike).
    for env in ["mcp_timeout=2400000", "mcp_idle=0", "mcp_background=3600000"] {
        assert!(pane_contains(&srv.fx, &a.id, env).await, "{env} not in the agent's environment");
    }
}

#[tokio::test]
async fn a_decision_after_the_held_call_is_gone_is_typed_into_the_session() {
    let srv = start().await;
    let tools = tools_repo(&srv);
    let a = spawn(&srv, "x").await;
    let t = token(&srv, &a.id).await;
    let task = request_in_background(&srv, &a.id, &t, tools.to_str().unwrap());
    wait_pending(&srv.fx, &a.id).await;
    assert!(wait_until(|| async { srv.fx.svc.is_holding(&a.id) }).await);
    task.abort();
    let _ = task.await;
    // Until the server drops the handler a decision would still go to the (dead) call instead of being typed.
    assert!(wait_until(|| async { !srv.fx.svc.is_holding(&a.id) }).await);
    let (status, v) = grant(&srv, &a.id, tools.to_str().unwrap()).await;
    assert_eq!(status, 200, "{v}");
    let wt = PathBuf::from(&a.workdir).join("tools");
    assert!(pane_contains(&srv.fx, &a.id, &format!("fleet-board: granted tools at {} (branch fleet/x)", wt.display())).await);
    assert!(wt.join("README").is_file());
    let now = agent(&srv.fx, &a.id).await;
    assert_eq!((now.status, now.repo_request), (Status::Working, None));

    let task = request_in_background(&srv, &a.id, &t, "elsewhere");
    wait_pending(&srv.fx, &a.id).await;
    assert!(wait_until(|| async { srv.fx.svc.is_holding(&a.id) }).await);
    task.abort();
    let _ = task.await;
    assert!(wait_until(|| async { !srv.fx.svc.is_holding(&a.id) }).await);
    assert_eq!(refuse(&srv, &a.id, "no").await.0, 204);
    assert!(pane_contains(&srv.fx, &a.id, "fleet-board: refused elsewhere: no").await);
}

#[tokio::test]
async fn a_hold_past_its_limit_ends_the_call_and_the_decision_is_typed() {
    let fx = fixture().await;
    let mut opts = fx.svc.options().clone();
    opts.repo_request_hold = Duration::from_millis(300);
    let svc = Arc::new(AgentService::new(fx.svc.config().clone(), fx.store.clone(), fx.repos.clone(), fx.tmux.clone(), opts));
    let srv = serve_with(fx, svc.clone()).await;
    let req = SpawnRequest { actor: Actor("local".into()), repos: vec!["core".into()], prompt: "x".into(), task_slug: None };
    let a = svc.spawn(req).await.unwrap();
    assert!(pane_contains(&srv.fx, &a.id, "ready").await);
    let t = token(&srv, &a.id).await;
    let (_, v) = rpc_at(srv.addr, &a.id, &t, &call("request_repo", json!({"repo": "elsewhere", "why": "w"}))).await;
    let (text, err) = result_of(&v);
    assert!(err && text.contains("typed into this session"), "{text}");
    assert!(agent(&srv.fx, &a.id).await.repo_request.is_some(), "the request stays on the card");
    svc.refuse_repo(&a.id, "later").await.unwrap();
    assert!(pane_contains(&srv.fx, &a.id, "fleet-board: refused elsewhere: later").await);
}

#[tokio::test]
async fn kill_ends_a_held_call() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    let t = token(&srv, &a.id).await;
    let task = request_in_background(&srv, &a.id, &t, "tools");
    wait_pending(&srv.fx, &a.id).await;
    srv.fx.svc.kill(&a.id).await.unwrap();
    let (_, v) = tokio::time::timeout(Duration::from_secs(10), task).await.unwrap().unwrap();
    assert!(result_of(&v).1, "{v}");
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
    let task = request_in_background(&srv, &a.id, &t, "tools");
    wait_pending(&srv.fx, &a.id).await;
    assert_eq!(refuse(&srv, &a.id, "no").await.0, 204);
    let _ = tokio::time::timeout(Duration::from_secs(10), task).await;
    let mut seen = vec![];
    while let Ok(e) = rx.try_recv() {
        if let EventKind::StatusChanged { id, repo_request, .. } = e.kind {
            if id == a.id {
                seen.push(repo_request);
            }
        }
    }
    assert_eq!(seen.first(), Some(&Some(RepoRequest { repo: "tools".into(), why: "need it".into() })), "{seen:?}");
    assert_eq!(seen.last(), Some(&None), "{seen:?}");
}

/// The fallback types the decision only when the card was waiting on the request itself, read before the decision
/// moves it to Working: anything else may be a dialog that Enter would answer.
#[tokio::test]
async fn the_fallback_types_only_onto_the_request_it_answers() {
    let fx = fixture().await;
    let mut opts = fx.svc.options().clone();
    opts.repo_request_hold = Duration::from_millis(100);
    let svc = Arc::new(AgentService::new(fx.svc.config().clone(), fx.store.clone(), fx.repos.clone(), fx.tmux.clone(), opts));
    let req = SpawnRequest { actor: Actor("local".into()), repos: vec!["core".into()], prompt: "x".into(), task_slug: None };
    let a = svc.spawn(req).await.unwrap();
    assert!(pane_contains(&fx, &a.id, "ready").await);
    let tools = make_repo(&fx._tmp.path().join("tools"));
    assert!(svc.request_repo(&a.id, tools.to_str().unwrap(), "w").await.is_err(), "the hold runs out");
    svc.grant_repo(&a.id, tools.to_str().unwrap()).await.unwrap();
    assert!(pane_contains(&fx, &a.id, "fleet-board: granted tools at").await);

    assert!(svc.request_repo(&a.id, "elsewhere", "w").await.is_err());
    // A permission prompt came up meanwhile (as the hooks would record it).
    fx.store.update_agent(&a.id, |x| x.blocked_reason = Some(BlockedReason::Permission)).await.unwrap();
    svc.refuse_repo(&a.id, "no").await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let pane = fx.tmux.capture(&session_name(&a.id)).await.unwrap();
    assert!(!pane.contains("refused elsewhere"), "{pane}");
}
