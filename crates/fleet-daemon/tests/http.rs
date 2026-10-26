mod common;

use common::net::*;
use common::*;
use fleet_api::{
    default_ui_schema, openapi, Agent, AgentId, ArchivePlan, ArchiveReport, ProjectList,
    ProjectState, Snapshot, Status,
};
use fleet_daemon::agents::{CLAUDE_MD_HEADER, PROMPT_FILE};
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
    let state = AppState::new(
        fx.svc.clone(),
        fx.store.clone(),
        fx.tmux.clone(),
        addr.port(),
    );
    let probe = state.clone();
    tokio::spawn(async move { axum::serve(listener, router(state)).await });
    Server {
        addr,
        fx,
        state: probe,
    }
}

async fn start() -> Server {
    serve(fixture().await).await
}

fn spawn_body(project: &str, prompt: &str) -> Value {
    json!({"actor": "local", "repos": [project], "prompt": prompt, "task_slug": null, "persona": PERSONA})
}

async fn spawn(srv: &Server, prompt: &str) -> Agent {
    let (status, v) = json_of(
        srv.addr,
        "POST",
        "/api/agents",
        Some(&spawn_body("core", prompt)),
    )
    .await;
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["warnings"], json!([]), "{v}");
    serde_json::from_value(v["agent"].clone()).unwrap()
}

#[tokio::test]
async fn spawn_then_snapshot_contains_the_agent() {
    let srv = start().await;
    let a = spawn(&srv, "fix it").await;
    let (status, v) = json_of(srv.addr, "GET", "/api/agents", None).await;
    let snap: Snapshot = serde_json::from_value(v).unwrap();
    assert_eq!(status, 200);
    assert_eq!(
        snap.agents.iter().map(|x| &x.id).collect::<Vec<_>>(),
        [&a.id]
    );
}

#[tokio::test]
async fn spawn_errors_are_json_with_the_right_status() {
    let srv = start().await;
    let (s1, v1) = json_of(
        srv.addr,
        "POST",
        "/api/agents",
        Some(&spawn_body("nope", "x")),
    )
    .await;
    assert_eq!(s1, 400);
    assert!(v1["error"].as_str().unwrap().contains("nope"), "{v1}");
    let (s2, _) = json_of(
        srv.addr,
        "POST",
        "/api/agents",
        Some(&spawn_body("core", "  ")),
    )
    .await;
    assert_eq!(s2, 400);
    let (s3, _) = http(srv.addr, "POST", "/api/agents", &[], None).await;
    assert!(s3 == 400 || s3 == 415 || s3 == 422, "{s3}");
}

#[tokio::test]
async fn unknown_agent_is_404_and_dead_agent_is_409() {
    let srv = start().await;
    for (m, p, b) in [
        (
            "POST",
            "/api/agents/zz/answer",
            json!({"actor": "local", "text": "x"}),
        ),
        ("POST", "/api/agents/zz/pause", json!({"actor": "local"})),
        ("POST", "/api/agents/zz/resume", json!({"actor": "local"})),
        ("POST", "/api/agents/zz/stop", json!(null)),
    ] {
        let (status, _) = http(srv.addr, m, p, &[], Some(&b)).await;
        assert_eq!(status, 404, "{m} {p}");
    }
    let (t, _) = http(srv.addr, "GET", "/api/agents/zz/terminal", &[], None).await;
    assert_eq!(t, 404);

    let a = spawn(&srv, "x").await;
    srv.fx.tmux.kill(&session_name(&a.id)).await.unwrap();
    srv.fx.svc.reconcile().await.unwrap();
    let (status, _) = http(
        srv.addr,
        "POST",
        &format!("/api/agents/{}/answer", a.id.0),
        &[],
        Some(&json!({"actor": "local", "text": "x"})),
    )
    .await;
    assert_eq!(status, 409);
}

#[tokio::test]
async fn answer_pause_resume_and_stop_work_over_http() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    assert!(pane_contains(&srv.fx, &a.id, "ready").await);
    let base = format!("/api/agents/{}", a.id.0);
    let ctl = json!({"actor": "local"});
    assert_eq!(
        http(
            srv.addr,
            "POST",
            &format!("{base}/answer"),
            &[],
            Some(&json!({"actor": "local", "text": "hello there"}))
        )
        .await
        .0,
        204
    );
    assert!(pane_contains(&srv.fx, &a.id, "hello there").await);
    assert_eq!(
        http(srv.addr, "POST", &format!("{base}/pause"), &[], Some(&ctl))
            .await
            .0,
        204
    );
    assert!(agent(&srv.fx, &a.id).await.paused);
    assert_eq!(
        http(srv.addr, "POST", &format!("{base}/resume"), &[], Some(&ctl))
            .await
            .0,
        204
    );
    assert!(!agent(&srv.fx, &a.id).await.paused);
    assert_eq!(
        http(srv.addr, "POST", &format!("{base}/stop"), &[], None)
            .await
            .0,
        204
    );
    assert_eq!(agent(&srv.fx, &a.id).await.status, Status::Stopped);
}

#[tokio::test]
async fn delete_agent_is_gone_and_post_stop_stops() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    let base = format!("/api/agents/{}", a.id.0);
    assert_eq!(http(srv.addr, "DELETE", &base, &[], None).await.0, 405);
    assert_eq!(
        srv.fx.store.snapshot().await.unwrap().agents.len(),
        1,
        "the refused DELETE must not remove anything"
    );
    assert_eq!(
        http(srv.addr, "POST", &format!("{base}/stop"), &[], None)
            .await
            .0,
        204
    );
    assert!(!srv.fx.tmux.has_session(&session_name(&a.id)).await.unwrap());
}

#[tokio::test]
async fn the_terminal_of_a_stopped_agent_sends_its_saved_screen_then_closed() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    assert!(pane_contains(&srv.fx, &a.id, "ready").await);
    srv.fx.svc.stop(&a.id).await.unwrap();
    let saved = srv
        .fx
        .store
        .project(&a.id)
        .await
        .unwrap()
        .unwrap()
        .last_screen
        .unwrap();
    // The stream ends by itself, so the plain request returns with the whole body.
    let (status, body) = http(
        srv.addr,
        "GET",
        &format!("/api/agents/{}/terminal", a.id.0),
        &[],
        None,
    )
    .await;
    assert_eq!(status, 200);
    // `closed`, as for a session that ends live, so the clients stop reconnecting and say "Session ended".
    let events: Vec<&str> = body.lines().filter(|l| l.starts_with("event:")).collect();
    assert_eq!(events, ["event: pane", "event: closed"], "{body}");
    let data = body
        .lines()
        .find_map(|l| l.strip_prefix("data:"))
        .expect("a data line");
    assert_eq!(serde_json::from_str::<String>(data.trim()).unwrap(), saved);
}

#[tokio::test]
async fn the_diff_of_a_stopped_agent_still_works() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    std::fs::write(
        PathBuf::from(&a.workspaces[0].worktree).join("new.txt"),
        "hello\n",
    )
    .unwrap();
    srv.fx.svc.stop(&a.id).await.unwrap();
    let (s, v) = json_of(
        srv.addr,
        "GET",
        &format!("/api/agents/{}/diff", a.id.0),
        None,
    )
    .await;
    assert_eq!(s, 200, "{v}");
    assert!(v["workspaces"][0]["diff"]
        .as_str()
        .unwrap()
        .contains("+hello"));
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
    assert_eq!(
        (first.event.as_deref(), first.id.as_deref()),
        (Some("event"), Some("2"))
    );
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
        assert_eq!(
            first.event.as_deref(),
            Some("snapshot"),
            "Last-Event-ID {last}"
        );
        assert_eq!(
            serde_json::from_str::<Snapshot>(&first.data).unwrap().seq,
            5
        );
    }
}

#[tokio::test]
async fn hook_route_applies_events_and_rejects_bad_callers() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    let token = srv.fx.store.agent_token(&a.id).await.unwrap().unwrap();
    let body: Value = serde_json::from_str(
        &std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/hooks/Stop-1.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let path = format!("/internal/hooks/{}", a.id.0);
    assert_eq!(
        http(
            srv.addr,
            "POST",
            &path,
            &[("X-Fleet-Token", &token)],
            Some(&body)
        )
        .await
        .0,
        200
    );
    assert_eq!(agent(&srv.fx, &a.id).await.status, Status::Review);
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
    assert_eq!(
        http(
            srv.addr,
            "POST",
            "/internal/hooks/nobody",
            &[("X-Fleet-Token", &token)],
            Some(&body)
        )
        .await
        .0,
        404
    );
    assert_eq!(
        http(
            srv.addr,
            "POST",
            &path,
            &[("X-Fleet-Token", &token)],
            Some(&json!({"x": 1}))
        )
        .await
        .0,
        400
    );
}

#[tokio::test]
async fn an_exit_reported_by_hook_triggers_a_measurement() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    let token = srv.fx.store.agent_token(&a.id).await.unwrap().unwrap();
    let (status, v) = http(
        srv.addr,
        "POST",
        &format!("/internal/hooks/{}", a.id.0),
        &[("X-Fleet-Token", &token)],
        Some(&json!({"hook_event_name": "SessionEnd"})),
    )
    .await;
    assert_eq!(status, 200, "{v}");
    assert_eq!(agent(&srv.fx, &a.id).await.status, Status::Stopped);
    assert!(
        wait_until(|| async { agent(&srv.fx, &a.id).await.disk_bytes.is_some() }).await,
        "the exited project was never measured"
    );
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
        if serde_json::from_str::<String>(&ev.data)
            .unwrap()
            .contains("ready")
        {
            seen = true;
            break;
        }
    }
    assert!(seen, "pane text never reached the stream");
    for m in ["POST", "PUT", "DELETE"] {
        assert_eq!(
            http(srv.addr, m, &path, &[], Some(&json!({"text": "rm -rf /"})))
                .await
                .0,
            405,
            "{m}"
        );
    }
}

#[tokio::test]
async fn a_quiet_event_stream_sends_named_pings() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let fx = fixture().await;
    let listener = bind_loopback("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = AppState::new(
        fx.svc.clone(),
        fx.store.clone(),
        fx.tmux.clone(),
        addr.port(),
    )
    .with_keep_alive(std::time::Duration::from_millis(100));
    tokio::spawn(async move { axum::serve(listener, router(state)).await });
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(format!("GET /api/events HTTP/1.0\r\nHost: {addr}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut seen = String::new();
    let mut buf = [0u8; 4096];
    let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
    // A browser's EventSource never surfaces comments, so the keep-alive must be an event it can listen for.
    while !seen.contains("event: ping") && std::time::Instant::now() < end {
        let n = tokio::time::timeout(std::time::Duration::from_secs(1), s.read(&mut buf))
            .await
            .unwrap_or(Ok(0))
            .unwrap();
        seen.push_str(&String::from_utf8_lossy(&buf[..n]));
    }
    assert!(seen.contains("event: ping"), "{seen}");
}

#[tokio::test]
async fn get_repos_lists_configured_repos() {
    let srv = start().await;
    let (s, v) = json_of(srv.addr, "GET", "/api/repos", None).await;
    assert_eq!(s, 200);
    assert_eq!(v["repos"][0]["name"], "core");
    assert_eq!(v["repos"][0]["source"], "path");
    assert_eq!(v["repos"].as_array().unwrap().len(), 1);
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
    std::fs::write(
        PathBuf::from(&a.workspaces[0].worktree).join("new.txt"),
        "hello\n",
    )
    .unwrap();
    let (s, v) = json_of(
        srv.addr,
        "GET",
        &format!("/api/agents/{}/diff", a.id.0),
        None,
    )
    .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["workspaces"][0]["repo"], "core");
    assert!(v["workspaces"][0]["diff"]
        .as_str()
        .unwrap()
        .contains("+hello"));
    assert_eq!(
        http(srv.addr, "GET", "/api/agents/zz/diff", &[], None)
            .await
            .0,
        404
    );
}

#[tokio::test]
async fn terminal_stream_task_ends_when_the_client_goes_away() {
    // The pane of an idle agent never changes, so the task only notices a gone client if it looks for it.
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    assert!(pane_contains(&srv.fx, &a.id, "ready").await);
    let path = format!("/api/agents/{}/terminal", a.id.0);
    let mut clients = vec![
        SseClient::connect(srv.addr, &path, &[]).await,
        SseClient::connect(srv.addr, &path, &[]).await,
    ];
    for c in &mut clients {
        c.next().await;
    }
    assert_eq!(srv.state.active_terminal_streams(), 2);
    drop(clients);
    assert!(
        wait_until(|| async { srv.state.active_terminal_streams() == 0 }).await,
        "terminal tasks leaked"
    );
}

#[tokio::test]
async fn ui_schema_and_openapi_routes_serve_the_contract() {
    let srv = start().await;
    let (s, v) = json_of(srv.addr, "GET", "/api/ui-schema", None).await;
    assert_eq!(
        (s, v),
        (200, serde_json::to_value(default_ui_schema()).unwrap())
    );
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
        assert_eq!(
            http(
                srv.addr,
                "GET",
                p,
                &[("Host", &format!("evil.example:{port}"))],
                None
            )
            .await
            .0,
            403,
            "{p}"
        );
    }
    assert_eq!(
        http(
            srv.addr,
            "POST",
            "/api/agents",
            &[("Host", "evil.example")],
            Some(&body)
        )
        .await
        .0,
        403
    );
    // A page on another local port or site posting to us.
    for origin in ["http://evil.example", "http://localhost:3000", "null"] {
        let (s, _) = http(
            srv.addr,
            "POST",
            "/api/agents",
            &[("Origin", origin)],
            Some(&body),
        )
        .await;
        assert_eq!(s, 403, "{origin}");
    }
    assert!(
        srv.fx.store.snapshot().await.unwrap().agents.is_empty(),
        "a refused request still spawned an agent"
    );
    // Same-origin and non-browser callers are fine.
    let same = format!("http://127.0.0.1:{port}");
    assert_eq!(
        http(srv.addr, "GET", "/api/agents", &[("Origin", &same)], None)
            .await
            .0,
        200
    );
    assert_eq!(
        http(
            srv.addr,
            "GET",
            "/api/agents",
            &[("Host", &format!("localhost:{port}"))],
            None
        )
        .await
        .0,
        200
    );
    assert_eq!(
        http(
            srv.addr,
            "GET",
            "/api/agents",
            &[("Host", &format!("[::1]:{port}"))],
            None
        )
        .await
        .0,
        200
    );
}

#[tokio::test]
async fn the_daemon_refuses_to_bind_a_non_loopback_address() {
    for addr in ["0.0.0.0:0", "[::]:0"] {
        assert!(
            bind_loopback(addr.parse().unwrap()).await.is_err(),
            "{addr}"
        );
    }
    assert!(bind_loopback("127.0.0.1:0".parse().unwrap()).await.is_ok());
}

#[tokio::test]
async fn http_repos_lists_remembered_repos() {
    let srv = start().await;
    let tools = make_repo(&srv.fx._tmp.path().join("tools"));
    srv.fx
        .repos
        .resolve(tools.to_str().unwrap(), &fixture_env(&srv.fx).await)
        .await
        .unwrap();
    let (status, v) = json_of(srv.addr, "GET", "/api/repos", None).await;
    assert_eq!(status, 200, "{v}");
    let list: fleet_api::RepoList = serde_json::from_value(v).unwrap();
    let names: Vec<_> = list
        .repos
        .iter()
        .map(|r| (r.name.as_str(), r.source))
        .collect();
    assert_eq!(
        names,
        [
            ("core", fleet_api::RepoSource::Path),
            ("tools", fleet_api::RepoSource::Path)
        ]
    );
    assert_eq!(
        list.repos[1].location,
        std::fs::canonicalize(&tools).unwrap().to_string_lossy()
    );
}

#[tokio::test]
async fn http_repos_hides_credentials_a_remembered_url_already_has() {
    let srv = start().await;
    let row = fleet_daemon::store::RepoRow {
        name: "old".into(),
        source: fleet_api::RepoSource::Url,
        location: "https://user:secret@example.invalid/org/old.git".into(),
        base: srv.fx._tmp.path().join("old"),
        persona: None,
        roster: vec![],
    };
    srv.fx.store.insert_repo(&row).await.unwrap();
    let (status, v) = json_of(srv.addr, "GET", "/api/repos", None).await;
    assert_eq!(status, 200, "{v}");
    assert!(!v.to_string().contains("secret"), "{v}");
    assert_eq!(
        v["repos"][1]["location"],
        "https://example.invalid/org/old.git"
    );
}

#[tokio::test]
async fn http_add_repo_returns_the_workspace() {
    let srv = start().await;
    let tools = make_repo(&srv.fx._tmp.path().join("tools"));
    let a = spawn(&srv, "add one").await;
    let path = format!("/api/agents/{}/repos", a.id.0);
    let (status, v) = json_of(
        srv.addr,
        "POST",
        &path,
        Some(&json!({"actor": "local", "repo": tools.to_str().unwrap()})),
    )
    .await;
    assert_eq!(status, 200, "{v}");
    let w: fleet_api::Workspace = serde_json::from_value(v).unwrap();
    assert_eq!(
        (w.repo.as_str(), w.branch.as_str()),
        ("tools", "fleet/add-one")
    );
    assert_eq!(agent(&srv.fx, &a.id).await.workspaces.len(), 2);
    let (bad, v) = json_of(
        srv.addr,
        "POST",
        &path,
        Some(&json!({"actor": "local", "repo": "no such thing"})),
    )
    .await;
    assert_eq!(bad, 400, "{v}");
    let other = make_repo(&srv.fx._tmp.path().join("other"));
    git(&other, &["branch", "fleet/add-one"]);
    let (clash, v) = json_of(
        srv.addr,
        "POST",
        &path,
        Some(&json!({"actor": "local", "repo": other.to_str().unwrap()})),
    )
    .await;
    assert_eq!(clash, 409, "{v}");
    assert!(
        v["error"]
            .as_str()
            .unwrap()
            .contains("already exists in other"),
        "{v}"
    );
    let (gone, _) = json_of(
        srv.addr,
        "POST",
        "/api/agents/zz/repos",
        Some(&json!({"actor": "local", "repo": "core"})),
    )
    .await;
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
    git(
        srv.fx._tmp.path(),
        &[
            "clone",
            "-q",
            "--bare",
            seed.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
    let row = srv
        .fx
        .repos
        .resolve(
            &format!("file://{}", bare.display()),
            &fixture_env(&srv.fx).await,
        )
        .await
        .unwrap()
        .row;
    git(
        &row.base,
        &[
            "remote",
            "set-url",
            "origin",
            // https: a persona's git may use no other remote transport. The TLS handshake waits on this listener.
            &format!(
                "https://127.0.0.1:{}/slow.git",
                hang.local_addr().unwrap().port()
            ),
        ],
    );
    // The daemon's git inherits the environment, and a proxy there would take the fake remote's connection. An
    // empty http.proxy in this repo's own config turns proxying off for its fetch alone, unlike a process-wide
    // env change, which races the other tests' getenv and leaks into them.
    git(&row.base, &["config", "http.proxy", ""]);
    let body = spawn_body(&row.name, "slow one").to_string();
    let request = format!(
        "POST /api/agents HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        srv.addr.port(),
        body.len()
    );
    let mut client = tokio::net::TcpStream::connect(srv.addr).await.unwrap();
    client.write_all(request.as_bytes()).await.unwrap();
    // The fetch reaching the remote means the spawn is mid-way: the project folder exists, the worktree does not.
    // Polled against a deadline: a git that never connects (a refused protocol, a proxy) must fail the test, not hang it.
    hang.set_nonblocking(true).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let fetch = loop {
        match hang.accept() {
            Ok((conn, _)) => break conn,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "the fetch never reached the fake remote"
                );
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Err(e) => panic!("accept: {e}"),
        }
    };
    drop(client);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    drop(fetch);
    assert!(
        wait_until(|| async { srv.fx.store.snapshot().await.unwrap().agents.len() == 1 }).await,
        "the spawn stopped with the client: {:?}",
        std::fs::read_dir(&srv.fx.projects_root)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect::<Vec<_>>()
    );
    // The agent row lands before its tmux session does. Dropping the fixture in between would kill a server that is
    // not there yet, and the late `new-session` would start one nothing ever kills.
    let id = srv.fx.store.snapshot().await.unwrap().agents[0].id.clone();
    let name = session_name(&id);
    assert!(
        wait_until(|| async { srv.fx.tmux.has_session(&name).await.unwrap_or(false) }).await,
        "the spawn never started its session"
    );
}

async fn stop_over_http(srv: &Server, a: &Agent) {
    let (status, v) = json_of(
        srv.addr,
        "POST",
        &format!("/api/agents/{}/stop", a.id.0),
        None,
    )
    .await;
    assert_eq!(status, 204, "{v}");
}

async fn project_list(srv: &Server) -> ProjectList {
    let (status, v) = json_of(srv.addr, "GET", "/api/projects", None).await;
    assert_eq!(status, 200, "{v}");
    serde_json::from_value(v).unwrap()
}

/// A folder the daemon made and lost the record of.
fn lost_folder(srv: &Server, name: &str) -> PathBuf {
    let folder = srv.fx.projects_root.join(name);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("CLAUDE.md"),
        format!("{CLAUDE_MD_HEADER}\nmore\n"),
    )
    .unwrap();
    std::fs::write(folder.join(PROMPT_FILE), "the task\n").unwrap();
    folder
}

#[tokio::test]
async fn projects_lists_archived_and_orphans() {
    let srv = start().await;
    let a = spawn(&srv, "old work").await;
    stop_over_http(&srv, &a).await;
    let (status, plan) = json_of(
        srv.addr,
        "GET",
        &format!("/api/projects/{}/archive-plan", a.id.0),
        None,
    )
    .await;
    assert_eq!(status, 200, "{plan}");
    let (status, v) = json_of(
        srv.addr,
        "POST",
        &format!("/api/projects/{}/archive", a.id.0),
        Some(&json!({"items": plan["items"]})),
    )
    .await;
    assert_eq!(status, 200, "{v}");
    let report: ArchiveReport = serde_json::from_value(v).unwrap();
    assert_eq!(report.kept, None);

    let folder = lost_folder(&srv, "lost");
    let (status, _) = json_of(srv.addr, "POST", "/api/projects/refresh", None).await;
    assert_eq!(status, 202);
    let id = format!("project_folder:{}", folder.display());
    let mut list = project_list(&srv).await;
    for _ in 0..200 {
        if list.orphans.iter().any(|o| o.id == id) && list.clones_bytes.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        list = project_list(&srv).await;
    }
    assert!(list.orphans.iter().any(|o| o.id == id), "{list:?}");
    assert!(list.clones_bytes.is_some(), "{list:?}");
    let p = list.projects.iter().find(|p| p.id == a.id).unwrap();
    assert_eq!(p.state, ProjectState::Archived);
}

#[tokio::test]
async fn clones_bytes_is_none_until_measured_and_never_walks_on_the_request() {
    let srv = start().await;
    assert_eq!(project_list(&srv).await.clones_bytes, None);
}

#[tokio::test]
async fn archive_on_a_running_project_is_409() {
    let srv = start().await;
    let a = spawn(&srv, "busy").await;
    let nothing = json!({"items": []});
    for (method, path, body) in [
        (
            "GET",
            format!("/api/projects/{}/archive-plan", a.id.0),
            None,
        ),
        (
            "POST",
            format!("/api/projects/{}/archive", a.id.0),
            Some(&nothing),
        ),
    ] {
        let (status, v) = json_of(srv.addr, method, &path, body).await;
        assert_eq!(status, 409, "{path}: {v}");
        assert_eq!(v["error"], "the project is not stopped; stop it first");
    }
    let (status, _) = json_of(srv.addr, "GET", "/api/projects/nope/archive-plan", None).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn orphan_routes_plan_archive_and_end_sessions() {
    let srv = start().await;
    let folder = lost_folder(&srv, "lost");
    srv.fx
        .tmux
        .new_session(
            "fleet-stray",
            &srv.fx.state_dir,
            &[],
            &["sleep".into(), "60".into()],
        )
        .await
        .unwrap();
    srv.fx.svc.rescan_orphans().await;
    let id = format!("project_folder:{}", folder.display());

    let (status, v) = json_of(
        srv.addr,
        "POST",
        "/api/orphans/plan",
        Some(&json!({"id": id})),
    )
    .await;
    assert_eq!(status, 200, "{v}");
    let plan: ArchivePlan = serde_json::from_value(v).unwrap();

    // A live session is ended, never archived; unknown ids are 404 on every orphan route.
    let (status, _) = json_of(
        srv.addr,
        "POST",
        "/api/orphans/archive",
        Some(&json!({"id": "session:fleet-stray", "items": []})),
    )
    .await;
    assert_eq!(status, 409);
    for path in ["plan", "archive", "end-session"] {
        let (status, _) = json_of(
            srv.addr,
            "POST",
            &format!("/api/orphans/{path}"),
            Some(&json!({"id": "worktree:/nowhere", "items": []})),
        )
        .await;
        assert_eq!(status, 404, "{path}");
    }

    let (status, v) = json_of(
        srv.addr,
        "POST",
        "/api/orphans/end-session",
        Some(&json!({"id": "session:fleet-stray"})),
    )
    .await;
    assert_eq!(status, 204, "{v}");
    assert!(!srv.fx.tmux.has_session("fleet-stray").await.unwrap());

    // A file the plan listed is gone: the confirmed plan no longer holds, and nothing goes.
    std::fs::remove_file(folder.join(PROMPT_FILE)).unwrap();
    let (status, v) = json_of(
        srv.addr,
        "POST",
        "/api/orphans/archive",
        Some(&json!({"id": id, "items": plan.items})),
    )
    .await;
    assert_eq!(status, 409, "{v}");
    assert_eq!(v["error"], fleet_api::PLAN_CHANGED);
    assert!(folder.join("CLAUDE.md").is_file());

    let (_, fresh) = json_of(
        srv.addr,
        "POST",
        "/api/orphans/plan",
        Some(&json!({"id": id})),
    )
    .await;
    let (status, v) = json_of(
        srv.addr,
        "POST",
        "/api/orphans/archive",
        Some(&json!({"id": id, "items": fresh["items"]})),
    )
    .await;
    assert_eq!(status, 200, "{v}");
    assert!(!folder.exists());
    assert!(srv.fx.svc.orphans().is_empty());
}

#[tokio::test]
async fn two_quick_refreshes_queue_one_pass() {
    // Current-thread runtime: nothing the service spawns runs until this test awaits, so both requests land
    // while the first is still queued.
    let srv = start().await;
    let svc = srv.fx.svc.clone();
    svc.request_measure_all();
    svc.request_measure_all();
    assert!(wait_until(|| async { svc.measure_all_passes() >= 1 }).await);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(svc.measure_all_passes(), 1);
    // Once the pass is over a new request is a new pass, not swallowed by a stale flag.
    svc.request_measure_all();
    assert!(wait_until(|| async { svc.measure_all_passes() == 2 }).await);

    let rescans = svc.rescans_run();
    svc.request_rescan();
    svc.request_rescan();
    assert!(wait_until(|| async { svc.rescans_run() > rescans }).await);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(svc.rescans_run(), rescans + 1);
    svc.request_rescan();
    assert!(wait_until(|| async { svc.rescans_run() == rescans + 2 }).await);
}

#[tokio::test]
async fn persona_routes_map_errors() {
    let srv = start().await;
    let body = json!({"name": "Home", "host": "https://GitHub.com/", "commit_name": "T", "commit_email": "t@e", "github_auth": "gh"});
    let (s, v) = json_of(srv.addr, "POST", "/api/personas", Some(&body)).await;
    assert_eq!(
        (s, v["id"].as_str(), v["host"].as_str()),
        (200, Some("home"), Some("github.com"))
    );
    let bad = json!({"name": "X", "host": "a b", "commit_name": "T", "commit_email": "t@e", "github_auth": "gh"});
    assert_eq!(
        json_of(srv.addr, "POST", "/api/personas", Some(&bad))
            .await
            .0,
        400
    );
    for (name, email) in [(" ", "t@e"), ("T", ""), ("T", " \t")] {
        let bad = json!({"name": "X", "host": "github.com", "commit_name": name, "commit_email": email, "github_auth": "gh"});
        let (s, v) = json_of(srv.addr, "POST", "/api/personas", Some(&bad)).await;
        assert_eq!(s, 400, "{name:?} {email:?}: {v}");
    }
    for patch in [json!({"commit_name": "  "}), json!({"commit_email": ""})] {
        let (s, v) = json_of(srv.addr, "PATCH", "/api/personas/home", Some(&patch)).await;
        assert_eq!(s, 400, "{patch}: {v}");
        assert!(v.to_string().contains("commit name and email"), "{v}");
    }
    let (s, v) = json_of(
        srv.addr,
        "PATCH",
        "/api/personas/home",
        Some(&json!({"name": "Renamed"})),
    )
    .await;
    assert_eq!((s, v["name"].as_str()), (200, Some("Renamed")));
    assert_eq!(
        json_of(srv.addr, "PATCH", "/api/personas/nope", Some(&json!({})))
            .await
            .0,
        404
    );
    assert_eq!(
        json_of(srv.addr, "POST", "/api/personas/nope/check", None)
            .await
            .0,
        404
    );
    assert_eq!(
        json_of(srv.addr, "DELETE", "/api/personas/nope", None)
            .await
            .0,
        404
    );
    let (s, v) = json_of(srv.addr, "GET", "/api/personas", None).await;
    // The fixture's own persona and this one.
    assert_eq!((s, v["personas"].as_array().map(Vec::len)), (200, Some(2)));

    let mut a = agent_fixture(&srv.fx);
    a.id = AgentId("a1".into());
    a.persona = "home".into();
    srv.fx.store.insert_agent(&a, "t").await.unwrap();
    let (s, v) = json_of(srv.addr, "DELETE", "/api/personas/home", None).await;
    assert_eq!(s, 409, "{v}");
    srv.fx
        .store
        .archive_project(&a.id, |p| p.state = ProjectState::Archived)
        .await
        .unwrap();
    assert_eq!(
        json_of(srv.addr, "DELETE", "/api/personas/home", None)
            .await
            .0,
        204
    );
}

#[tokio::test]
async fn spawn_persona_errors_are_400() {
    let srv = start().await;
    let mut body = spawn_body("core", "x");
    for (persona, needle) in [("", "pick a persona"), ("nobody", "no such persona")] {
        body["persona"] = json!(persona);
        let (s, v) = json_of(srv.addr, "POST", "/api/agents", Some(&body)).await;
        assert_eq!(s, 400, "{persona:?}: {v}");
        assert!(v["error"].as_str().unwrap().contains(needle), "{v}");
    }
    let gh = srv.fx._tmp.path().join("personas").join(PERSONA).join("gh");
    std::fs::remove_file(gh.join("hosts.yml")).unwrap();
    body["persona"] = json!(PERSONA);
    let (s, v) = json_of(srv.addr, "POST", "/api/agents", Some(&body)).await;
    assert_eq!(s, 400, "{v}");
    assert!(v["error"].as_str().unwrap().contains("GitHub again"), "{v}");
}

#[tokio::test]
async fn http_repos_filter_by_persona() {
    let srv = start().await;
    let other = ready_persona(&srv.fx.svc, "other", "github.com").await;
    let tools = make_repo(&srv.fx._tmp.path().join("tools"));
    let row = fleet_daemon::store::RepoRow {
        name: "tools".into(),
        source: fleet_api::RepoSource::Path,
        location: tools.to_string_lossy().into_owned(),
        base: tools.clone(),
        persona: Some(other.clone()),
        roster: vec![],
    };
    srv.fx.store.insert_repo(&row).await.unwrap();
    let names = |v: &Value| -> Vec<(String, Value)> {
        v["repos"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r["name"].as_str().unwrap().to_string(),
                    r["persona"].clone(),
                )
            })
            .collect()
    };
    let (s, v) = json_of(srv.addr, "GET", "/api/repos", None).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(
        names(&v),
        [
            ("core".to_string(), json!(PERSONA)),
            ("tools".to_string(), json!(other))
        ]
    );
    let (s, v) = json_of(
        srv.addr,
        "GET",
        &format!("/api/repos?persona={other}"),
        None,
    )
    .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(names(&v), [("tools".to_string(), json!(other))]);
    let (_, v) = json_of(srv.addr, "GET", "/api/repos?persona=nobody", None).await;
    assert_eq!(names(&v), []);
}

#[tokio::test]
async fn agent_config_round_trips_and_rejects_invalid() {
    let srv = start().await;
    let (status, v) = json_of(srv.addr, "GET", "/api/agent-config", None).await;
    assert_eq!(status, 200, "{v}");
    assert_eq!(
        v,
        json!({"instructions": "", "settings": {"allow": [], "deny": [], "env": {}}, "settings_error": null})
    );
    let body = json!({"instructions": "hi", "settings": {"allow": ["Read"], "deny": [], "env": {"A": "b"}}});
    let (status, v) = json_of(srv.addr, "PUT", "/api/agent-config", Some(&body)).await;
    assert_eq!(status, 200, "{v}");
    let want = json!({"instructions": "hi", "settings": {"allow": ["Read"], "deny": [], "env": {"A": "b"}}, "settings_error": null});
    assert_eq!(v, want);
    let (_, v) = json_of(srv.addr, "GET", "/api/agent-config", None).await;
    assert_eq!(v, want);

    let bad = json!({"instructions": "", "settings": {"allow": [], "deny": [], "env": {"GH_TOKEN": "x"}}});
    let (status, v) = json_of(srv.addr, "PUT", "/api/agent-config", Some(&bad)).await;
    assert_eq!(status, 400, "{v}");
    assert!(v.to_string().contains("GH_TOKEN"), "{v}");
    let (_, v) = json_of(srv.addr, "GET", "/api/agent-config", None).await;
    assert_eq!(v, want, "a rejected save leaves the old config");
    let (status, v) = json_of(srv.addr, "PUT", "/api/agent-config", Some(&json!({}))).await;
    assert_eq!(status, 400, "{v}");
}

#[tokio::test]
async fn a_broken_settings_file_is_reported_and_repaired_without_touching_instructions() {
    let srv = start().await;
    let dir = srv.fx.state_dir.parent().unwrap().join("agent");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("instructions.md"), "keep me").unwrap();
    std::fs::write(dir.join("settings.json"), "{").unwrap();
    let (status, v) = json_of(srv.addr, "GET", "/api/agent-config", None).await;
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["instructions"], "keep me");
    assert!(
        v["settings_error"]
            .as_str()
            .unwrap()
            .contains("settings.json"),
        "{v}"
    );

    // An instructions save leaves the broken file alone, and the error stays reported.
    let (status, v) = json_of(
        srv.addr,
        "PUT",
        "/api/agent-config",
        Some(&json!({"instructions": "new"})),
    )
    .await;
    assert_eq!(status, 200, "{v}");
    assert_eq!(
        std::fs::read_to_string(dir.join("settings.json")).unwrap(),
        "{"
    );
    assert!(v["settings_error"].is_string(), "{v}");

    // A settings save repairs the file and leaves the instructions byte for byte.
    let (status, v) = json_of(
        srv.addr,
        "PUT",
        "/api/agent-config",
        Some(&json!({"settings": {"allow": ["Read"], "deny": [], "env": {}}})),
    )
    .await;
    assert_eq!(status, 200, "{v}");
    assert!(v["settings_error"].is_null(), "{v}");
    assert_eq!(std::fs::read(dir.join("instructions.md")).unwrap(), b"new");
}

/// A request whose body is plain text, as a definition is saved.
async fn text_of(srv: &Server, method: &str, path: &str, body: &str) -> (u16, Value) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(srv.addr).await.unwrap();
    let req = format!(
        "{method} {path} HTTP/1.0\r\nHost: 127.0.0.1:{}\r\nContent-Type: text/markdown\r\nContent-Length: {}\r\n\r\n{body}",
        srv.addr.port(),
        body.len()
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut raw = String::new();
    s.read_to_string(&mut raw).await.unwrap();
    let status = raw.split_whitespace().nth(1).unwrap().parse().unwrap();
    let text = raw.split_once("\r\n\r\n").map(|x| x.1).unwrap_or("");
    (
        status,
        serde_json::from_str(text).unwrap_or(Value::String(text.into())),
    )
}

#[tokio::test]
async fn definition_routes_read_write_and_reset() {
    let srv = start().await;
    let (s, v) = json_of(srv.addr, "GET", "/api/definitions", None).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v.as_array().unwrap().len(), 12, "{v}");

    let (s, v) = json_of(srv.addr, "GET", "/api/definitions/specialist/rust", None).await;
    assert_eq!((s, v["builtin"].as_bool()), (200, Some(true)), "{v}");
    assert!(v["text"].as_str().unwrap().starts_with("---\nname: rust\n"));
    assert_eq!(
        json_of(srv.addr, "GET", "/api/definitions/specialist/nope", None)
            .await
            .0,
        404
    );

    let bad = "---\nname: rust\nsummary: s\nlimit: {}\n---\nbody\n";
    let (s, v) = text_of(&srv, "PUT", "/api/definitions/specialist/rust", bad).await;
    assert_eq!(s, 400, "{v}");
    assert_eq!(v["error"], "specialists/rust.md: unknown key `limit`");

    // Nothing to reset yet: the built-in itself cannot be deleted.
    assert_eq!(
        json_of(srv.addr, "DELETE", "/api/definitions/specialist/rust", None)
            .await
            .0,
        404
    );

    let good = "---\nname: rust\nsummary: Mine\n---\nbody\n";
    let (s, v) = text_of(&srv, "PUT", "/api/definitions/specialist/rust", good).await;
    assert_eq!((s, v["overridden"].as_bool()), (200, Some(true)), "{v}");
    assert!(srv
        .fx
        ._tmp
        .path()
        .join("fleet/specialists/rust.md")
        .is_file());
    assert_eq!(
        json_of(srv.addr, "DELETE", "/api/definitions/specialist/rust", None)
            .await
            .0,
        204
    );
    assert!(!srv
        .fx
        ._tmp
        .path()
        .join("fleet/specialists/rust.md")
        .exists());
}

#[tokio::test]
async fn bad_roster_name_is_400() {
    let srv = start().await;
    let path = format!("/api/repos/{PERSONA}/core/fleet");
    let (s, v) = json_of(
        srv.addr,
        "PUT",
        &path,
        Some(&json!({"roster": ["rust", "nope"]})),
    )
    .await;
    assert_eq!(s, 400, "{v}");
    assert!(v["error"].as_str().unwrap().contains("nope"), "{v}");

    let (s, v) = json_of(
        srv.addr,
        "PUT",
        &path,
        Some(&json!({"roster": ["rust", "reviewer"]})),
    )
    .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["name"], "core", "{v}");
    assert_eq!(v["roster"], json!(["rust", "reviewer"]), "{v}");

    let (s, v) = json_of(srv.addr, "PUT", &path, Some(&json!({"roster": ["rust"]}))).await;
    assert_eq!(s, 200, "{v}");
    let (_, v) = json_of(
        srv.addr,
        "GET",
        &format!("/api/repos?persona={PERSONA}"),
        None,
    )
    .await;
    assert_eq!(v["repos"][0]["roster"], json!(["rust"]), "{v}");

    let (s, v) = json_of(
        srv.addr,
        "PUT",
        &format!("/api/repos/{PERSONA}/nope/fleet"),
        Some(&json!({"roster": []})),
    )
    .await;
    assert_eq!(s, 404, "{v}");
}

/// A path repo `name` remembered for the fixture persona, whose origin's `main` has `workflow` when given. Pushed,
/// so `origin/main` is there without a fetch.
async fn remembered_with_origin(srv: &Server, name: &str, workflow: Option<&str>) {
    let tmp = srv.fx._tmp.path();
    let dir = make_repo(&tmp.join(name));
    if let Some(yaml) = workflow {
        std::fs::create_dir_all(dir.join(".fleet")).unwrap();
        std::fs::write(dir.join(".fleet/workflow.yml"), yaml).unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "workflow"]);
    }
    let bare = tmp.join(format!("{name}.git"));
    git(tmp, &["init", "-q", "--bare", bare.to_str().unwrap()]);
    let url = format!("file://{}", bare.display());
    git(&dir, &["remote", "add", "origin", &url]);
    git(&dir, &["push", "-q", "origin", "main"]);
    let row = fleet_daemon::store::RepoRow {
        name: name.into(),
        source: fleet_api::RepoSource::Path,
        location: dir.to_string_lossy().into_owned(),
        base: dir.clone(),
        persona: Some(PERSONA.into()),
        roster: vec![],
    };
    srv.fx.store.insert_repo(&row).await.unwrap();
}

#[tokio::test]
async fn repo_list_reports_workflow_state() {
    let srv = start().await;
    remembered_with_origin(&srv, "found", Some("steps:\n  fmt:\n    run: cargo fmt\n")).await;
    remembered_with_origin(&srv, "missing", None).await;
    remembered_with_origin(&srv, "invalid", Some("steps: {}\n")).await;
    let (s, v) = json_of(
        srv.addr,
        "GET",
        &format!("/api/repos?persona={PERSONA}"),
        None,
    )
    .await;
    assert_eq!(s, 200, "{v}");
    let workflow = |name: &str| -> Value {
        v["repos"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["name"] == name)
            .unwrap_or_else(|| panic!("{name} not listed: {v}"))["workflow"]
            .clone()
    };
    assert_eq!(
        workflow("found"),
        json!({"state": "found", "base": "main", "steps": ["fmt"]})
    );
    assert_eq!(
        workflow("missing"),
        json!({"state": "missing", "base": "main"})
    );
    assert_eq!(
        workflow("invalid"),
        json!({"state": "invalid", "base": "main", "error": "`steps` is empty"})
    );
    // The fixture's `core` has no origin: nothing to read the file from, which is not the same as no file.
    let core = workflow("core");
    assert_eq!(
        (core["state"].as_str(), core["base"].as_str()),
        (Some("unreadable"), Some("main")),
        "{core}"
    );
    assert!(!core["error"].as_str().unwrap().is_empty(), "{core}");
}

#[tokio::test]
async fn an_archive_that_refuses_the_folder_is_409_with_the_reason() {
    let srv = start().await;
    let a = spawn(&srv, "relinked").await;
    stop_over_http(&srv, &a).await;
    let workdir = PathBuf::from(&a.workdir);
    let elsewhere = srv.fx._tmp.path().join("elsewhere");
    std::fs::rename(&workdir, &elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &workdir).unwrap();
    let (status, v) = json_of(
        srv.addr,
        "POST",
        &format!("/api/projects/{}/archive", a.id.0),
        Some(&json!({"items": []})),
    )
    .await;
    assert_eq!(status, 409, "{v}");
    assert!(
        v["error"].as_str().unwrap().contains("symbolic link"),
        "{v}"
    );
}
