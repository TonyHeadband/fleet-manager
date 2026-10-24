mod common;

use common::net::*;
use common::*;
use fleet_api::{default_ui_schema, openapi, Agent, AgentId, Snapshot, Status};
use fleet_daemon::http::{bind_loopback, router, AppState};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::PathBuf;

struct Server {
    addr: SocketAddr,
    fx: Fx,
    state: AppState,
}

async fn serve(fx: Fx) -> Server {
    let listener = bind_loopback("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = AppState::new(fx.svc.clone(), fx.store.clone(), fx.tmux.clone(), addr.port());
    let probe = state.clone();
    tokio::spawn(async move { axum::serve(listener, router(state)).await });
    Server { addr, fx, state: probe }
}

async fn start() -> Server {
    serve(fixture().await).await
}

fn spawn_body(project: &str, prompt: &str) -> Value {
    json!({"actor": "local", "repos": [project], "prompt": prompt, "task_slug": null})
}

async fn spawn(srv: &Server, prompt: &str) -> Agent {
    let (status, v) = json_of(srv.addr, "POST", "/api/agents", Some(&spawn_body("core", prompt))).await;
    assert_eq!(status, 200, "{v}");
    serde_json::from_value(v).unwrap()
}

#[tokio::test]
async fn spawn_then_snapshot_contains_the_agent() {
    let srv = start().await;
    let a = spawn(&srv, "fix it").await;
    let (status, v) = json_of(srv.addr, "GET", "/api/agents", None).await;
    let snap: Snapshot = serde_json::from_value(v).unwrap();
    assert_eq!(status, 200);
    assert_eq!(snap.agents.iter().map(|x| &x.id).collect::<Vec<_>>(), [&a.id]);
}

#[tokio::test]
async fn spawn_errors_are_json_with_the_right_status() {
    let srv = start().await;
    let (s1, v1) = json_of(srv.addr, "POST", "/api/agents", Some(&spawn_body("nope", "x"))).await;
    assert_eq!(s1, 400);
    assert!(v1["error"].as_str().unwrap().contains("nope"), "{v1}");
    let (s2, _) = json_of(srv.addr, "POST", "/api/agents", Some(&spawn_body("core", "  "))).await;
    assert_eq!(s2, 400);
    let (s3, _) = http(srv.addr, "POST", "/api/agents", &[], None).await;
    assert!(s3 == 400 || s3 == 415 || s3 == 422, "{s3}");
}

#[tokio::test]
async fn unknown_agent_is_404_and_dead_agent_is_409() {
    let srv = start().await;
    for (m, p, b) in [
        ("POST", "/api/agents/zz/answer", json!({"actor": "local", "text": "x"})),
        ("POST", "/api/agents/zz/pause", json!({"actor": "local"})),
        ("POST", "/api/agents/zz/resume", json!({"actor": "local"})),
        ("DELETE", "/api/agents/zz", json!(null)),
    ] {
        let (status, _) = http(srv.addr, m, p, &[], if m == "DELETE" { None } else { Some(&b) }).await;
        assert_eq!(status, 404, "{m} {p}");
    }
    let (t, _) = http(srv.addr, "GET", "/api/agents/zz/terminal", &[], None).await;
    assert_eq!(t, 404);

    let a = spawn(&srv, "x").await;
    srv.fx.tmux.kill(&session_name(&a.id)).await.unwrap();
    srv.fx.svc.reconcile().await.unwrap();
    let (status, _) = http(srv.addr, "POST", &format!("/api/agents/{}/answer", a.id.0), &[], Some(&json!({"actor": "local", "text": "x"}))).await;
    assert_eq!(status, 409);
}

#[tokio::test]
async fn answer_pause_resume_and_kill_work_over_http() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    assert!(pane_contains(&srv.fx, &a.id, "ready").await);
    let base = format!("/api/agents/{}", a.id.0);
    let ctl = json!({"actor": "local"});
    assert_eq!(http(srv.addr, "POST", &format!("{base}/answer"), &[], Some(&json!({"actor": "local", "text": "hello there"}))).await.0, 204);
    assert!(pane_contains(&srv.fx, &a.id, "hello there").await);
    assert_eq!(http(srv.addr, "POST", &format!("{base}/pause"), &[], Some(&ctl)).await.0, 204);
    assert!(agent(&srv.fx, &a.id).await.paused);
    assert_eq!(http(srv.addr, "POST", &format!("{base}/resume"), &[], Some(&ctl)).await.0, 204);
    assert!(!agent(&srv.fx, &a.id).await.paused);
    let (status, v) = json_of(srv.addr, "DELETE", &base, None).await;
    assert_eq!((status, v), (200, json!({"kept": null})));
    assert!(srv.fx.store.snapshot().await.unwrap().agents.is_empty());
}

#[tokio::test]
async fn event_stream_starts_with_a_snapshot_then_delivers_seq_ordered_events() {
    let srv = start().await;
    let mut c = SseClient::connect(srv.addr, "/api/events", &[]).await;
    let first = c.next().await;
    assert_eq!(first.event.as_deref(), Some("snapshot"));
    let snap: Snapshot = serde_json::from_str(&first.data).unwrap();
    let a = spawn(&srv, "x").await;
    let ev = c.next().await;
    assert_eq!(ev.event.as_deref(), Some("event"));
    let v: Value = serde_json::from_str(&ev.data).unwrap();
    assert_eq!(v["seq"], snap.seq + 1);
    assert_eq!(ev.id.as_deref(), Some((snap.seq + 1).to_string().as_str()));
    assert_eq!(v["kind"]["type"], "agent_created");
    assert_eq!(v["kind"]["agent"]["id"], a.id.0);
}

#[tokio::test]
async fn reconnect_with_last_event_id_gets_only_the_missed_events() {
    let srv = start().await;
    let a = spawn(&srv, "one").await;
    let b = spawn(&srv, "two").await;
    let mut c = SseClient::connect(srv.addr, "/api/events", &[("Last-Event-ID", "1")]).await;
    let first = c.next().await;
    assert_eq!((first.event.as_deref(), first.id.as_deref()), (Some("event"), Some("2")));
    let v: Value = serde_json::from_str(&first.data).unwrap();
    assert_eq!(v["kind"]["agent"]["id"], b.id.0);
    let _ = a;
}

#[tokio::test]
async fn reconnect_beyond_the_retained_log_or_from_the_future_gets_a_snapshot() {
    let srv = serve(fixture_retaining(2).await).await;
    for i in 0..5 {
        let mut a = serde_json::from_value::<Agent>(json!({
            "id": format!("x{i}"), "task": "t", "workdir": "/w", "tmux_session": "s",
            "workspaces": [{"repo": "core", "branch": "b", "worktree": "/w/core", "base": "abc", "has_changes": false}],
            "status": "queued", "activity": "", "status_since": 1, "has_changes": false, "paused": false,
            "owner": "o", "blocked_reason": null
        }))
        .unwrap();
        a.id = AgentId(format!("x{i}"));
        srv.fx.store.insert_agent(&a, "t").await.unwrap();
    }
    for last in ["1", "99"] {
        let mut c = SseClient::connect(srv.addr, "/api/events", &[("Last-Event-ID", last)]).await;
        let first = c.next().await;
        assert_eq!(first.event.as_deref(), Some("snapshot"), "Last-Event-ID {last}");
        assert_eq!(serde_json::from_str::<Snapshot>(&first.data).unwrap().seq, 5);
    }
}

#[tokio::test]
async fn hook_route_applies_events_and_rejects_bad_callers() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    let token = srv.fx.store.agent_token(&a.id).await.unwrap().unwrap();
    let body: Value = serde_json::from_str(&std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/hooks/Stop-1.json")).unwrap()).unwrap();
    let path = format!("/internal/hooks/{}", a.id.0);
    assert_eq!(http(srv.addr, "POST", &path, &[("X-Fleet-Token", &token)], Some(&body)).await.0, 200);
    assert_eq!(agent(&srv.fx, &a.id).await.status, Status::Review);
    assert_eq!(http(srv.addr, "POST", &path, &[("X-Fleet-Token", "wrong")], Some(&body)).await.0, 401);
    assert_eq!(http(srv.addr, "POST", &path, &[], Some(&body)).await.0, 401);
    assert_eq!(http(srv.addr, "POST", "/internal/hooks/nobody", &[("X-Fleet-Token", &token)], Some(&body)).await.0, 404);
    assert_eq!(http(srv.addr, "POST", &path, &[("X-Fleet-Token", &token)], Some(&json!({"x": 1}))).await.0, 400);
}

#[tokio::test]
async fn terminal_stream_shows_the_pane_and_has_no_write_route() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    let path = format!("/api/agents/{}/terminal", a.id.0);
    let mut c = SseClient::connect(srv.addr, &path, &[]).await;
    let mut seen = false;
    for _ in 0..20 {
        let ev = c.next().await;
        assert_eq!(ev.event.as_deref(), Some("pane"));
        if serde_json::from_str::<String>(&ev.data).unwrap().contains("ready") {
            seen = true;
            break;
        }
    }
    assert!(seen, "pane text never reached the stream");
    for m in ["POST", "PUT", "DELETE"] {
        assert_eq!(http(srv.addr, m, &path, &[], Some(&json!({"text": "rm -rf /"}))).await.0, 405, "{m}");
    }
}

#[tokio::test]
async fn get_repos_lists_configured_repos() {
    let srv = start().await;
    let (s, v) = json_of(srv.addr, "GET", "/api/repos", None).await;
    assert_eq!(s, 200);
    assert_eq!(v["repos"][0]["name"], "core");
    assert_eq!(v["repos"][0]["source"], "path");
    assert_eq!(v["repos"].as_array().unwrap().len(), 1);
    assert_eq!(http(srv.addr, "GET", "/api/projects", &[], None).await.0, 404, "the old route is gone");
}

#[tokio::test]
async fn spawn_with_repo_field() {
    let srv = start().await;
    let a = spawn(&srv, "go").await;
    assert_eq!(a.workspaces[0].repo, "core");
    // An old field name is refused, never read as a project with no repos (`repos` defaults to empty).
    for field in ["repo", "project"] {
        let old = json!({"actor": "local", field: "core", "prompt": "go", "task_slug": null});
        let (s, text) = http(srv.addr, "POST", "/api/agents", &[], Some(&old)).await;
        assert_eq!(s, 422, "{text}");
        assert!(text.contains(&format!("unknown field `{field}`")), "{text}");
    }
    assert_eq!(srv.fx.store.snapshot().await.unwrap().agents.len(), 1);
}

#[tokio::test]
async fn diff_route_returns_each_workspaces_changes() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    std::fs::write(PathBuf::from(&a.workspaces[0].worktree).join("new.txt"), "hello\n").unwrap();
    let (s, v) = json_of(srv.addr, "GET", &format!("/api/agents/{}/diff", a.id.0), None).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["workspaces"][0]["repo"], "core");
    assert!(v["workspaces"][0]["diff"].as_str().unwrap().contains("+hello"));
    assert_eq!(http(srv.addr, "GET", "/api/agents/zz/diff", &[], None).await.0, 404);
}

#[tokio::test]
async fn terminal_stream_task_ends_when_the_client_goes_away() {
    // The pane of an idle agent never changes, so the task only notices a gone client if it looks for it.
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    assert!(pane_contains(&srv.fx, &a.id, "ready").await);
    let path = format!("/api/agents/{}/terminal", a.id.0);
    let mut clients = vec![SseClient::connect(srv.addr, &path, &[]).await, SseClient::connect(srv.addr, &path, &[]).await];
    for c in &mut clients {
        c.next().await;
    }
    assert_eq!(srv.state.active_terminal_streams(), 2);
    drop(clients);
    assert!(wait_until(|| async { srv.state.active_terminal_streams() == 0 }).await, "terminal tasks leaked");
}

#[tokio::test]
async fn ui_schema_and_openapi_routes_serve_the_contract() {
    let srv = start().await;
    let (s, v) = json_of(srv.addr, "GET", "/api/ui-schema", None).await;
    assert_eq!((s, v), (200, serde_json::to_value(default_ui_schema()).unwrap()));
    let (s, v) = json_of(srv.addr, "GET", "/api/openapi.json", None).await;
    assert_eq!((s, v), (200, serde_json::to_value(openapi()).unwrap()));
}

#[tokio::test]
async fn requests_with_a_foreign_host_or_origin_are_refused() {
    let srv = start().await;
    let port = srv.addr.port();
    let body = spawn_body("core", "x");
    // DNS rebinding: the browser sends the attacker's name as Host.
    for p in ["/api/agents", "/api/ui-schema", "/api/events"] {
        assert_eq!(http(srv.addr, "GET", p, &[("Host", &format!("evil.example:{port}"))], None).await.0, 403, "{p}");
    }
    assert_eq!(http(srv.addr, "POST", "/api/agents", &[("Host", "evil.example")], Some(&body)).await.0, 403);
    // A page on another local port or site posting to us.
    for origin in ["http://evil.example", "http://localhost:3000", "null"] {
        let (s, _) = http(srv.addr, "POST", "/api/agents", &[("Origin", origin)], Some(&body)).await;
        assert_eq!(s, 403, "{origin}");
    }
    assert!(srv.fx.store.snapshot().await.unwrap().agents.is_empty(), "a refused request still spawned an agent");
    // Same-origin and non-browser callers are fine.
    let same = format!("http://127.0.0.1:{port}");
    assert_eq!(http(srv.addr, "GET", "/api/agents", &[("Origin", &same)], None).await.0, 200);
    assert_eq!(http(srv.addr, "GET", "/api/agents", &[("Host", &format!("localhost:{port}"))], None).await.0, 200);
    assert_eq!(http(srv.addr, "GET", "/api/agents", &[("Host", &format!("[::1]:{port}"))], None).await.0, 200);
}

#[tokio::test]
async fn the_daemon_refuses_to_bind_a_non_loopback_address() {
    for addr in ["0.0.0.0:0", "[::]:0"] {
        assert!(bind_loopback(addr.parse().unwrap()).await.is_err(), "{addr}");
    }
    assert!(bind_loopback("127.0.0.1:0".parse().unwrap()).await.is_ok());
}

#[tokio::test]
async fn http_repos_lists_remembered_repos() {
    let srv = start().await;
    let tools = make_repo(&srv.fx._tmp.path().join("tools"));
    srv.fx.repos.resolve(tools.to_str().unwrap()).await.unwrap();
    let (status, v) = json_of(srv.addr, "GET", "/api/repos", None).await;
    assert_eq!(status, 200, "{v}");
    let list: fleet_api::RepoList = serde_json::from_value(v).unwrap();
    let names: Vec<_> = list.repos.iter().map(|r| (r.name.as_str(), r.source)).collect();
    assert_eq!(names, [("core", fleet_api::RepoSource::Path), ("tools", fleet_api::RepoSource::Path)]);
    assert_eq!(list.repos[1].location, std::fs::canonicalize(&tools).unwrap().to_string_lossy());
}

#[tokio::test]
async fn http_repos_hides_credentials_a_remembered_url_already_has() {
    let srv = start().await;
    let row = fleet_daemon::store::RepoRow {
        name: "old".into(),
        source: fleet_api::RepoSource::Url,
        location: "https://user:secret@example.invalid/org/old.git".into(),
        base: srv.fx._tmp.path().join("old"),
    };
    srv.fx.store.insert_repo(&row).await.unwrap();
    let (status, v) = json_of(srv.addr, "GET", "/api/repos", None).await;
    assert_eq!(status, 200, "{v}");
    assert!(!v.to_string().contains("secret"), "{v}");
    assert_eq!(v["repos"][1]["location"], "https://example.invalid/org/old.git");
}

#[tokio::test]
async fn http_add_repo_returns_the_workspace() {
    let srv = start().await;
    let tools = make_repo(&srv.fx._tmp.path().join("tools"));
    let a = spawn(&srv, "add one").await;
    let path = format!("/api/agents/{}/repos", a.id.0);
    let (status, v) = json_of(srv.addr, "POST", &path, Some(&json!({"actor": "local", "repo": tools.to_str().unwrap()}))).await;
    assert_eq!(status, 200, "{v}");
    let w: fleet_api::Workspace = serde_json::from_value(v).unwrap();
    assert_eq!((w.repo.as_str(), w.branch.as_str()), ("tools", "fleet/add-one"));
    assert_eq!(agent(&srv.fx, &a.id).await.workspaces.len(), 2);
    let (bad, v) = json_of(srv.addr, "POST", &path, Some(&json!({"actor": "local", "repo": "no such thing"}))).await;
    assert_eq!(bad, 400, "{v}");
    let other = make_repo(&srv.fx._tmp.path().join("other"));
    git(&other, &["branch", "fleet/add-one"]);
    let (clash, v) = json_of(srv.addr, "POST", &path, Some(&json!({"actor": "local", "repo": other.to_str().unwrap()}))).await;
    assert_eq!(clash, 409, "{v}");
    assert!(v["error"].as_str().unwrap().contains("already exists in other"), "{v}");
    let (gone, _) = json_of(srv.addr, "POST", "/api/agents/zz/repos", Some(&json!({"actor": "local", "repo": "core"}))).await;
    assert_eq!(gone, 404);
}

/// A client that hangs up (its timeout, a closed window) must not stop a spawn half-way: the project folder would
/// be left with no card to remove it with.
#[tokio::test]
async fn a_spawn_whose_client_hangs_up_mid_way_still_finishes() {
    use tokio::io::AsyncWriteExt;
    let srv = start().await;
    // Only a URL repo fetches. Its base is cloned from a local bare repo, then its origin is repointed at a remote
    // that accepts and never answers, which holds the attach's fetch for as long as the test wants.
    let hang = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let seed = make_repo(&srv.fx._tmp.path().join("slow-seed"));
    let bare = srv.fx._tmp.path().join("slow.git");
    git(srv.fx._tmp.path(), &["clone", "-q", "--bare", seed.to_str().unwrap(), bare.to_str().unwrap()]);
    let row = srv.fx.repos.resolve(&format!("file://{}", bare.display())).await.unwrap().row;
    git(&row.base, &["remote", "set-url", "origin", &format!("http://127.0.0.1:{}/slow.git", hang.local_addr().unwrap().port())]);
    let body = spawn_body(&row.name, "slow one").to_string();
    let request = format!(
        "POST /api/agents HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        srv.addr.port(),
        body.len()
    );
    let mut client = tokio::net::TcpStream::connect(srv.addr).await.unwrap();
    client.write_all(request.as_bytes()).await.unwrap();
    // The fetch reaching the remote means the spawn is mid-way: the project folder exists, the worktree does not.
    let (fetch, _) = tokio::task::spawn_blocking(move || hang.accept().unwrap()).await.unwrap();
    drop(client);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    drop(fetch);
    assert!(
        wait_until(|| async { srv.fx.store.snapshot().await.unwrap().agents.len() == 1 }).await,
        "the spawn stopped with the client: {:?}",
        std::fs::read_dir(&srv.fx.projects_root).unwrap().flatten().map(|e| e.file_name()).collect::<Vec<_>>()
    );
}
