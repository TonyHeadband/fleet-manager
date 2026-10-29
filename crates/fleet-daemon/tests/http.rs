mod common;

use common::net::*;
use common::*;
use fleet_api::{
    default_ui_schema, openapi, Agent, AgentId, ArchivePlan, ArchiveReport, Snapshot, Status,
    TaskList, TaskState,
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
    serve_on(fx, "127.0.0.1:0").await
}

async fn serve_on(fx: Fx, listen: &str) -> Server {
    let listener = bind_loopback(listen.parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = AppState::new(fx.svc.clone(), fx.store.clone(), fx.sessions.clone(), addr);
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

fn spawn_body(fx: &Fx, repo: &str, prompt: &str) -> Value {
    json!({"actor": "local", "product": PRODUCT, "repos": [pick(fx, repo)], "prompt": prompt, "slug": null, "persona": PERSONA})
}

async fn spawn(srv: &Server, prompt: &str) -> Agent {
    let (status, v) = json_of(
        srv.addr,
        "POST",
        "/api/agents",
        Some(&spawn_body(&srv.fx, "core", prompt)),
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
        Some(&spawn_body(&srv.fx, "nope", "x")),
    )
    .await;
    assert_eq!(s1, 400);
    assert!(v1["error"].as_str().unwrap().contains("nope"), "{v1}");
    let (s2, _) = json_of(
        srv.addr,
        "POST",
        "/api/agents",
        Some(&spawn_body(&srv.fx, "core", "  ")),
    )
    .await;
    assert_eq!(s2, 400);
    let (s3, _) = http(srv.addr, "POST", "/api/agents", &[], None).await;
    assert!(s3 == 400 || s3 == 415 || s3 == 422, "{s3}");
}

#[tokio::test]
async fn a_tool_refusal_is_424_with_the_tool_attached() {
    let srv = serve(fixture_with(vec!["fleet-no-such-claude".into()]).await).await;
    let (status, v) = json_of(
        srv.addr,
        "POST",
        "/api/agents",
        Some(&spawn_body(&srv.fx, "core", "x")),
    )
    .await;
    assert_eq!(status, 424, "{v}");
    assert_eq!(v["error"], "Claude Code is not installed", "{v}");
    assert_eq!(v["tool"]["kind"], "not-installed", "{v}");
    assert_eq!(v["tool"]["tool"], "claude", "{v}");
    // The client offers the fix from it.
    assert!(v["tool"]["fix"]["link"].is_string(), "{v}");
    // Every other refusal keeps its old body.
    let (_, other) = json_of(
        srv.addr,
        "POST",
        "/api/agents",
        Some(&spawn_body(&srv.fx, "core", "  ")),
    )
    .await;
    assert_eq!(other, json!({"error": "prompt is empty"}));
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
    srv.fx.sessions.kill(&session_name(&a.id)).unwrap();
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
    assert!(!srv.fx.sessions.alive(&session_name(&a.id)));
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
        .task(&a.id)
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
async fn the_terminal_of_a_running_agent_without_a_session_ends_without_closed() {
    // As during the resume after a daemon restart: not stopped, and no session yet.
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    assert!(pane_contains(&srv.fx, &a.id, "ready").await);
    srv.fx.sessions.kill(&a.session).unwrap();
    assert_ne!(
        srv.fx.svc.agent(&a.id).await.unwrap().status,
        fleet_api::Status::Stopped
    );
    let (status, body) = http(
        srv.addr,
        "GET",
        &format!("/api/agents/{}/terminal", a.id.0),
        &[],
        None,
    )
    .await;
    assert_eq!(status, 200);
    // No `closed`: the clients reconnect, and find the resumed session.
    assert!(!body.contains("event: closed"), "{body}");
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
            "id": format!("x{i}"), "goal": "t", "workdir": "/w", "session": "s",
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
        "the exited task was never measured"
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

/// An agent on `core` that has begun its burst: "ready" scrolls off at once, so `spawn_ready` would wait in vain.
async fn spawn_bursting(srv: &Server) -> Agent {
    let a = srv
        .fx
        .svc
        .spawn(fleet_api::SpawnRequest {
            actor: fleet_api::Actor("local".into()),
            product: PRODUCT.into(),
            release: None,
            repos: vec![pick(&srv.fx, "core")],
            prompt: "x".into(),
            slug: None,
            persona: PERSONA.into(),
            kind: fleet_api::AgentKind::Solo,
            role: None,
        })
        .await
        .unwrap();
    wait_screen_in(&srv.fx.sessions, &a.session, |s| s.contains("burst")).await;
    a
}

/// Reads `pane` events for `window`, returning their texts.
async fn panes_for(c: &mut SseClient, window: std::time::Duration) -> Vec<String> {
    let end = std::time::Instant::now() + window;
    let mut panes = vec![];
    while let Ok(ev) = tokio::time::timeout(
        end.saturating_duration_since(std::time::Instant::now()),
        c.next(),
    )
    .await
    {
        assert_eq!(ev.event.as_deref(), Some("pane"));
        panes.push(serde_json::from_str::<String>(&ev.data).unwrap());
    }
    panes
}

#[tokio::test]
async fn the_preview_follows_output_without_flooding() {
    let srv = serve(
        fixture_options(
            |_| {},
            |o| o.extra_env = vec![("FAKE_CLAUDE_BURST".into(), "2000".into())],
        )
        .await,
    )
    .await;
    let a = spawn_bursting(&srv).await;
    let path = format!("/api/agents/{}/terminal", a.id.0);
    let mut c = SseClient::connect(srv.addr, &path, &[]).await;
    let started = std::time::Instant::now();
    let panes = panes_for(&mut c, std::time::Duration::from_secs(2)).await;
    let secs = started.elapsed().as_secs_f64();
    assert!(
        panes.last().expect("a pane").contains("burst 2000"),
        "the last pane shows the end of the burst"
    );
    assert!(
        (panes.len() as f64) <= secs * 10.0 + 2.0,
        "{} panes in {secs:.1}s",
        panes.len()
    );
}

#[tokio::test]
async fn typing_shows_up_in_the_preview_within_a_moment() {
    let srv = start().await;
    let a = spawn_ready(&srv.fx).await;
    let path = format!("/api/agents/{}/terminal", a.id.0);
    let mut c = SseClient::connect(srv.addr, &path, &[]).await;
    async fn wait_pane(c: &mut SseClient, needle: &str) {
        loop {
            let ev = c.next().await;
            if serde_json::from_str::<String>(&ev.data)
                .unwrap()
                .contains(needle)
            {
                return;
            }
        }
    }
    wait_pane(&mut c, "ready").await;
    srv.fx.sessions.write_text(&a.session, "zzz").unwrap();
    let t = std::time::Instant::now();
    wait_pane(&mut c, "zzz").await;
    assert!(
        t.elapsed() < std::time::Duration::from_millis(400),
        "{:?}",
        t.elapsed()
    );
}

#[tokio::test]
async fn a_quiet_event_stream_sends_named_pings() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let fx = fixture().await;
    let listener = bind_loopback("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = AppState::new(fx.svc.clone(), fx.store.clone(), fx.sessions.clone(), addr)
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
    assert_eq!(v["repos"][0]["product"], PRODUCT);
    assert_eq!(v["repos"][0]["path"], json!(srv.fx.repo.to_string_lossy()));
    assert_eq!(v["repos"][0]["url"], Value::Null);
    assert_eq!(v["repos"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn spawn_with_repo_field() {
    let srv = start().await;
    let a = spawn(&srv, "go").await;
    assert_eq!(a.workspaces[0].repo, "core");
    // An old field name is refused, never read as a task with no repos (`repos` defaults to empty).
    for field in ["repo", "task"] {
        let old = json!({"actor": "local", field: "core", "prompt": "go", "slug": null});
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
    let body = spawn_body(&srv.fx, "core", "x");
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
        .resolve(
            PRODUCT,
            &pick(&srv.fx, tools.to_str().unwrap()),
            &fixture_env(&srv.fx).await,
        )
        .await
        .unwrap();
    let (status, v) = json_of(srv.addr, "GET", "/api/repos", None).await;
    assert_eq!(status, 200, "{v}");
    let list: fleet_api::RepoList = serde_json::from_value(v).unwrap();
    let names: Vec<_> = list
        .repos
        .iter()
        .map(|r| (r.name.as_str(), r.url.is_some()))
        .collect();
    assert_eq!(names, [("core", false), ("tools", false)]);
    assert_eq!(
        list.repos[1].path.as_deref(),
        Some(
            fleet_fs::canonical(&tools)
                .unwrap()
                .to_string_lossy()
                .as_ref()
        )
    );
}

#[tokio::test]
async fn http_repos_hides_credentials_a_remembered_url_already_has() {
    let srv = start().await;
    let row = fleet_daemon::store::RepoRow {
        product: PRODUCT.into(),
        name: "old".into(),
        path: None,
        url: Some("https://user:secret@example.invalid/org/old.git".into()),
    };
    srv.fx.store.upsert_repo(&row).await.unwrap();
    for path in ["/api/repos", "/api/products"] {
        let (status, v) = json_of(srv.addr, "GET", path, None).await;
        assert_eq!(status, 200, "{v}");
        assert!(!v.to_string().contains("secret"), "{path}: {v}");
    }
    let (_, v) = json_of(srv.addr, "GET", "/api/repos", None).await;
    assert_eq!(v["repos"][1]["url"], "https://example.invalid/org/old.git");
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

/// A client that hangs up (its timeout, a closed window) must not stop a spawn half-way: the task folder would
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
    let url = file_url(&bare);
    let resolved = srv
        .fx
        .repos
        .resolve(PRODUCT, &pick(&srv.fx, &url), &fixture_env(&srv.fx).await)
        .await
        .unwrap();
    let row = &resolved.row;
    // Rewritten for git's transport only: the clone still names the repo's URL, so it is still the repo's clone.
    // https: a persona's git may use no other remote transport. The TLS handshake waits on this listener.
    git(
        &resolved.base,
        &[
            "config",
            &format!(
                "url.https://127.0.0.1:{}/slow.git.insteadOf",
                hang.local_addr().unwrap().port()
            ),
            &url,
        ],
    );
    // The daemon's git inherits the environment, and a proxy there would take the fake remote's connection. An
    // empty http.proxy in this repo's own config turns proxying off for its fetch alone, unlike a process-wide
    // env change, which races the other tests' getenv and leaks into them.
    git(&resolved.base, &["config", "http.proxy", ""]);
    let body = spawn_body(&srv.fx, &row.name, "slow one").to_string();
    let request = format!(
        "POST /api/agents HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        srv.addr.port(),
        body.len()
    );
    let mut client = tokio::net::TcpStream::connect(srv.addr).await.unwrap();
    client.write_all(request.as_bytes()).await.unwrap();
    // The fetch reaching the remote means the spawn is mid-way: the task folder exists, the worktree does not.
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
        std::fs::read_dir(&srv.fx.tasks_root)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect::<Vec<_>>()
    );
    // The agent row lands before its session does. Dropping the fixture in between would end the sessions before
    // the late start, and nothing would ever end the one it starts.
    let id = srv.fx.store.snapshot().await.unwrap().agents[0].id.clone();
    let name = session_name(&id);
    assert!(
        wait_until(|| async { srv.fx.sessions.alive(&name) }).await,
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

async fn task_list(srv: &Server) -> TaskList {
    let (status, v) = json_of(srv.addr, "GET", "/api/tasks", None).await;
    assert_eq!(status, 200, "{v}");
    serde_json::from_value(v).unwrap()
}

/// A folder the daemon made and lost the record of.
fn lost_folder(srv: &Server, name: &str) -> PathBuf {
    let folder = srv.fx.tasks_root.join(name);
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
async fn tasks_lists_archived_and_orphans() {
    let srv = start().await;
    let a = spawn(&srv, "old work").await;
    stop_over_http(&srv, &a).await;
    let (status, plan) = json_of(
        srv.addr,
        "GET",
        &format!("/api/tasks/{}/archive-plan", a.id.0),
        None,
    )
    .await;
    assert_eq!(status, 200, "{plan}");
    let (status, v) = json_of(
        srv.addr,
        "POST",
        &format!("/api/tasks/{}/archive", a.id.0),
        Some(&json!({"items": plan["items"]})),
    )
    .await;
    assert_eq!(status, 200, "{v}");
    let report: ArchiveReport = serde_json::from_value(v).unwrap();
    assert_eq!(report.kept, None);

    let folder = lost_folder(&srv, "lost");
    let (status, _) = json_of(srv.addr, "POST", "/api/tasks/refresh", None).await;
    assert_eq!(status, 202);
    let id = format!("task_folder:{}", folder.display());
    let mut list = task_list(&srv).await;
    for _ in 0..200 {
        if list.orphans.iter().any(|o| o.id == id) && list.clones_bytes.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        list = task_list(&srv).await;
    }
    assert!(list.orphans.iter().any(|o| o.id == id), "{list:?}");
    assert!(list.clones_bytes.is_some(), "{list:?}");
    let p = list.tasks.iter().find(|p| p.id == a.id).unwrap();
    assert_eq!(p.state, TaskState::Archived);
}

#[tokio::test]
async fn clones_bytes_is_none_until_measured_and_never_walks_on_the_request() {
    let srv = start().await;
    assert_eq!(task_list(&srv).await.clones_bytes, None);
}

#[tokio::test]
async fn archive_on_a_running_task_is_409() {
    let srv = start().await;
    let a = spawn(&srv, "busy").await;
    let nothing = json!({"items": []});
    for (method, path, body) in [
        ("GET", format!("/api/tasks/{}/archive-plan", a.id.0), None),
        (
            "POST",
            format!("/api/tasks/{}/archive", a.id.0),
            Some(&nothing),
        ),
    ] {
        let (status, v) = json_of(srv.addr, method, &path, body).await;
        assert_eq!(status, 409, "{path}: {v}");
        assert_eq!(v["error"], "the task is not stopped; stop it first");
    }
    let (status, _) = json_of(srv.addr, "GET", "/api/tasks/nope/archive-plan", None).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn orphan_routes_plan_and_archive() {
    let srv = start().await;
    let folder = lost_folder(&srv, "lost");
    srv.fx.svc.rescan_orphans().await;
    let id = format!("task_folder:{}", folder.display());

    let (status, v) = json_of(
        srv.addr,
        "POST",
        "/api/orphans/plan",
        Some(&json!({"id": id})),
    )
    .await;
    assert_eq!(status, 200, "{v}");
    let plan: ArchivePlan = serde_json::from_value(v).unwrap();

    // Unknown ids are 404 on every orphan route.
    for path in ["plan", "archive"] {
        let (status, _) = json_of(
            srv.addr,
            "POST",
            &format!("/api/orphans/{path}"),
            Some(&json!({"id": "worktree:/nowhere", "items": []})),
        )
        .await;
        assert_eq!(status, 404, "{path}");
    }

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
        .archive_task(&a.id, |p| p.state = TaskState::Archived)
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
async fn spawn_persona_errors_are_400_or_a_tool_refusal() {
    let srv = start().await;
    let mut body = spawn_body(&srv.fx, "core", "x");
    for (persona, needle) in [("", "pick a persona"), ("nobody", "no such persona")] {
        body["persona"] = json!(persona);
        let (s, v) = json_of(srv.addr, "POST", "/api/agents", Some(&body)).await;
        assert_eq!(s, 400, "{persona:?}: {v}");
        assert!(v["error"].as_str().unwrap().contains(needle), "{v}");
    }
    let gh = srv.fx._tmp.path().join("personas").join(PERSONA).join("gh");
    std::fs::remove_file(gh.join("hosts.yml")).unwrap();
    body["persona"] = json!(PERSONA);
    // A persona logged out is a tool refusal.
    let (s, v) = json_of(srv.addr, "POST", "/api/agents", Some(&body)).await;
    assert_eq!(s, 424, "{v}");
    assert_eq!(
        v["tool"],
        json!({"kind": "not-logged-in", "tool": "gh", "persona": PERSONA}),
        "{v}"
    );
}

async fn add_product(srv: &Server, id: &str) {
    srv.fx
        .store
        .insert_product(&fleet_daemon::store::ProductRow {
            id: id.into(),
            name: format!("Product {id}"),
            default_persona: Some(PERSONA.into()),
            internal_url: None,
            created_at: 2,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn http_repos_filter_by_product() {
    let srv = start().await;
    add_product(&srv, "q").await;
    let tools = make_repo(&srv.fx._tmp.path().join("tools"));
    let row = fleet_daemon::store::RepoRow {
        product: "q".into(),
        name: "tools".into(),
        path: Some(tools.clone()),
        url: None,
    };
    srv.fx.store.upsert_repo(&row).await.unwrap();
    let names = |v: &Value| -> Vec<(String, Value)> {
        v["repos"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r["name"].as_str().unwrap().to_string(),
                    r["product"].clone(),
                )
            })
            .collect()
    };
    let (s, v) = json_of(srv.addr, "GET", "/api/repos", None).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(
        names(&v),
        [
            ("core".to_string(), json!(PRODUCT)),
            ("tools".to_string(), json!("q"))
        ]
    );
    let (s, v) = json_of(srv.addr, "GET", "/api/repos?product=q", None).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(names(&v), [("tools".to_string(), json!("q"))]);
    let (_, v) = json_of(srv.addr, "GET", "/api/repos?product=nobody", None).await;
    assert_eq!(names(&v), []);
}

#[tokio::test]
async fn get_products_lists_repos_releases_and_task_counts() {
    let srv = start().await;
    add_product(&srv, "q").await;
    let v1 = srv
        .fx
        .store
        .insert_release(&fleet_daemon::store::ReleaseRow {
            id: 0,
            product: "q".into(),
            version: "v1.0".into(),
            start: "2026-11-01".into(),
            target: "2026-12-15".into(),
            released_at: None,
        })
        .await
        .unwrap();
    spawn(&srv, "count me").await;
    let (s, v) = json_of(srv.addr, "GET", "/api/products", None).await;
    assert_eq!(s, 200, "{v}");
    let list: fleet_api::ProductList = serde_json::from_value(v).unwrap();
    let ids: Vec<_> = list.products.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, [PRODUCT, "q"]);
    let (p, q) = (&list.products[0], &list.products[1]);
    assert_eq!(p.tasks, 1);
    assert_eq!(
        p.repos.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        ["core"]
    );
    assert!(p.releases.is_empty());
    assert_eq!(
        (q.name.as_str(), q.default_persona.as_deref(), q.tasks),
        ("Product q", Some(PERSONA), 0)
    );
    assert_eq!(
        q.releases,
        [fleet_api::Release {
            id: v1,
            product: "q".into(),
            version: "v1.0".into(),
            start: "2026-11-01".into(),
            target: "2026-12-15".into(),
            released_at: None,
        }]
    );
}

#[tokio::test]
async fn spawn_product_errors_are_400() {
    let srv = start().await;
    add_product(&srv, "q").await;
    let theirs = srv
        .fx
        .store
        .insert_release(&fleet_daemon::store::ReleaseRow {
            id: 0,
            product: "q".into(),
            version: "v1".into(),
            start: "2026-11-01".into(),
            target: "2026-12-01".into(),
            released_at: None,
        })
        .await
        .unwrap();
    let mut body = spawn_body(&srv.fx, "core", "x");
    body["product"] = json!("nope");
    let (s, v) = json_of(srv.addr, "POST", "/api/agents", Some(&body)).await;
    assert_eq!(
        (s, v["error"].as_str()),
        (400, Some("no such product: nope")),
        "{v}"
    );
    let mut body = spawn_body(&srv.fx, "core", "x");
    body["release"] = json!(theirs);
    let (s, v) = json_of(srv.addr, "POST", "/api/agents", Some(&body)).await;
    assert_eq!(s, 400, "{v}");
    assert!(
        v["error"].as_str().unwrap().contains("not a release of p"),
        "{v}"
    );
    // A path-only repo picked with no checkout says what to do instead.
    let mut body = spawn_body(&srv.fx, "core", "x");
    body["repos"] = json!([{"repo": "core"}]);
    let (s, v) = json_of(srv.addr, "POST", "/api/agents", Some(&body)).await;
    assert_eq!(s, 400, "{v}");
    assert!(
        v["error"].as_str().unwrap().contains("core has no URL"),
        "{v}"
    );
    assert!(srv.fx.store.snapshot().await.unwrap().agents.is_empty());
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
    assert_eq!(v.as_array().unwrap().len(), 18, "{v}");

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
async fn hidden_specialists_are_saved_and_checked() {
    let srv = start().await;
    let path = format!("/api/products/{PRODUCT}/hidden-specialists");
    let (s, v) = json_of(
        srv.addr,
        "PUT",
        &path,
        Some(&json!({"hidden": ["rust", "nope"]})),
    )
    .await;
    assert_eq!(s, 400, "{v}");
    assert!(v["error"].as_str().unwrap().contains("nope"), "{v}");

    let (s, v) = json_of(
        srv.addr,
        "PUT",
        &path,
        Some(&json!({"hidden": ["reviewer", "docs"]})),
    )
    .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["id"], PRODUCT, "{v}");
    assert_eq!(v["hidden_specialists"], json!(["reviewer", "docs"]), "{v}");
    let (_, v) = json_of(srv.addr, "GET", "/api/products", None).await;
    assert_eq!(
        v["products"][0]["hidden_specialists"],
        json!(["reviewer", "docs"]),
        "{v}"
    );

    let (s, v) = json_of(
        srv.addr,
        "PUT",
        "/api/products/nope/hidden-specialists",
        Some(&json!({"hidden": []})),
    )
    .await;
    assert_eq!(s, 404, "{v}");
    // The repo roster route is gone.
    let (s, _) = json_of(
        srv.addr,
        "PUT",
        &format!("/api/products/{PRODUCT}/repos/core/fleet"),
        Some(&json!({"roster": []})),
    )
    .await;
    assert_eq!(s, 404);
}

/// A path repo `name` of the fixture product, whose origin's `main` has `workflow` when given. Pushed,
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
    git(
        tmp,
        &["init", "-q", "--bare", "-b", "main", bare.to_str().unwrap()],
    );
    let url = file_url(&bare);
    git(&dir, &["remote", "add", "origin", &url]);
    git(&dir, &["push", "-q", "origin", "main"]);
    add_path_repo(&srv.fx, name, &dir).await;
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
        &format!("/api/repos?product={PRODUCT}"),
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

// Creating a symlink needs a privilege on Windows (Developer Mode); sub-project 2 decides how to test it there.
#[cfg(unix)]
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
        &format!("/api/tasks/{}/archive", a.id.0),
        Some(&json!({"items": []})),
    )
    .await;
    assert_eq!(status, 409, "{v}");
    assert!(
        v["error"].as_str().unwrap().contains("symbolic link"),
        "{v}"
    );
}

#[tokio::test]
async fn agent_lists_its_attach_command() {
    let srv = start().await;
    let a = spawn(&srv, "x").await;
    let snapshot = || async {
        let (status, v) = json_of(srv.addr, "GET", "/api/agents", None).await;
        assert_eq!(status, 200, "{v}");
        serde_json::from_value::<Snapshot>(v)
            .unwrap()
            .agents
            .remove(0)
    };
    let live = snapshot().await;
    let argv = live
        .attach
        .expect("a running agent lists its attach command");
    assert!(
        argv[0].ends_with(&format!("fleet-attach{}", std::env::consts::EXE_SUFFIX)),
        "{argv:?}"
    );
    assert_eq!(
        argv[argv.len() - 3..],
        [
            "--url".to_string(),
            format!("http://{}", srv.addr),
            live.id.0.clone()
        ]
    );

    stop_over_http(&srv, &a).await;
    assert_eq!(
        snapshot().await.attach,
        None,
        "a stopped agent has no session"
    );
}

#[tokio::test]
async fn the_attach_command_names_the_address_the_daemon_listens_on() {
    let srv = serve_on(fixture().await, "[::1]:0").await;
    let a = spawn(&srv, "x").await;
    let (status, v) = json_of(srv.addr, "GET", "/api/agents", None).await;
    assert_eq!(status, 200, "{v}");
    let argv = serde_json::from_value::<Snapshot>(v)
        .unwrap()
        .agents
        .remove(0)
        .attach
        .expect("a running agent lists its attach command");
    assert_eq!(
        argv[argv.len() - 2..],
        [format!("http://[::1]:{}", srv.addr.port()), a.id.0.clone()]
    );
}

#[tokio::test]
async fn tasks_route_replaces_the_old_route() {
    let srv = start().await;
    let (status, v) = json_of(srv.addr, "GET", "/api/tasks", None).await;
    assert_eq!(status, 200, "{v}");
    let (status, _) = json_of(srv.addr, "GET", "/api/projects", None).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn shutdown_is_accepted_and_notifies_the_daemon() {
    let fx = fixture().await;
    let listener = bind_loopback("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let stop = std::sync::Arc::new(tokio::sync::Notify::new());
    let state = AppState::new(fx.svc.clone(), fx.store.clone(), fx.sessions.clone(), addr)
        .with_shutdown(stop.clone());
    tokio::spawn(async move { axum::serve(listener, router(state)).await });

    let (status, _) = http(addr, "POST", "/api/shutdown", &[], None).await;
    assert_eq!(status, 202);
    tokio::time::timeout(std::time::Duration::from_secs(2), stop.notified())
        .await
        .expect("the daemon was not told to stop");
}

#[tokio::test]
async fn shutdown_from_a_foreign_origin_is_refused() {
    let fx = fixture().await;
    let listener = bind_loopback("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let stop = std::sync::Arc::new(tokio::sync::Notify::new());
    let state = AppState::new(fx.svc.clone(), fx.store.clone(), fx.sessions.clone(), addr)
        .with_shutdown(stop.clone());
    tokio::spawn(async move { axum::serve(listener, router(state)).await });

    let (status, _) = http(
        addr,
        "POST",
        "/api/shutdown",
        &[("Origin", "http://evil.example")],
        None,
    )
    .await;
    assert_eq!(status, 403);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(300), stop.notified())
            .await
            .is_err(),
        "a refused request still stopped the daemon"
    );
}

#[tokio::test]
async fn tools_routes_answer_with_the_list() {
    let srv = start().await;
    let (status, v) = json_of(srv.addr, "GET", "/api/tools", None).await;
    assert_eq!(status, 200, "{v}");
    let tools = v["tools"].as_array().expect("a tools array");
    assert!(tools.iter().any(|t| t["tool"] == "gh"), "{v}");
    let (status, v) = json_of(srv.addr, "POST", "/api/tools/recheck", None).await;
    assert_eq!(status, 200, "{v}");
    assert!(v["tools"].is_array(), "{v}");
    let (status, v) = json_of(srv.addr, "GET", "/api/version", None).await;
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
}

async fn post_release(srv: &Server, body: Value) -> (u16, Value) {
    json_of(
        srv.addr,
        "POST",
        &format!("/api/products/{PRODUCT}/releases"),
        Some(&body),
    )
    .await
}

#[tokio::test]
async fn a_bad_release_names_its_field() {
    let srv = start().await;
    let (s, v) = post_release(
        &srv,
        json!({"version": "1", "start": "2026-10-01", "target": "soon"}),
    )
    .await;
    assert_eq!(s, 400, "{v}");
    assert_eq!(v["field"], "target", "{v}");
    assert_eq!(v["error"], "\"soon\" is not a date; use YYYY-MM-DD");
    let (s, v) = post_release(
        &srv,
        json!({"version": "  ", "start": "2026-10-01", "target": "2026-11-01"}),
    )
    .await;
    assert_eq!(s, 400, "{v}");
    assert_eq!(v["field"], "version", "{v}");
}

#[tokio::test]
async fn mark_released_still_passes_the_rule_check() {
    let srv = start().await;
    let (s, v) = post_release(
        &srv,
        json!({"version": "1.0", "start": "2026-10-01", "target": "2026-11-01"}),
    )
    .await;
    assert_eq!(s, 200, "{v}");
    let id = v["id"].as_i64().unwrap();
    let (s, v) = json_of(
        srv.addr,
        "PATCH",
        &format!("/api/releases/{id}"),
        Some(&json!({"released": true})),
    )
    .await;
    assert_eq!(s, 200, "{v}");
}

#[tokio::test]
async fn a_duplicate_version_has_no_field() {
    let srv = start().await;
    let good = json!({"version": "1.0", "start": "2026-10-01", "target": "2026-11-01"});
    let (s, _) = post_release(&srv, good.clone()).await;
    assert_eq!(s, 200);
    let (s, v) = post_release(&srv, good).await;
    assert_eq!(s, 409, "{v}");
    assert!(v.get("field").is_none(), "{v}");
}
