//! fleet-client against the real daemon router (the daemon crate owns this test so the dependency points one way).
mod common;

use common::net::http;
use common::*;
use fleet_api::{default_ui_schema, ActionDef, Agent, AgentId};
use fleet_client::http::FleetClient;
use fleet_client::stream::{spawn_event_stream, spawn_pane_stream, PaneMsg, StreamMsg};
use fleet_daemon::http::{bind_loopback, router, AppState};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::time::Duration;

async fn start() -> (String, Fx) {
    let fx = fixture().await;
    let listener = bind_loopback("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = AppState::new(
        fx.svc.clone(),
        fx.store.clone(),
        fx.tmux.clone(),
        addr.port(),
    );
    tokio::spawn(async move { axum::serve(listener, router(state)).await });
    (format!("http://{addr}"), fx)
}

/// The client is blocking by design (it runs on background threads in the GUI), so tests call it from a
/// blocking section of the multi-thread runtime.
macro_rules! blocking {
    ($c:ident, $body:expr) => {{
        tokio::task::block_in_place(|| $body)
    }};
}

fn action(id: &str) -> ActionDef {
    default_ui_schema()
        .actions
        .into_iter()
        .find(|a| a.id == id)
        .unwrap()
}

fn wake() -> Arc<dyn Fn() + Send + Sync> {
    Arc::new(|| {})
}

#[tokio::test(flavor = "multi_thread")]
async fn client_round_trips_against_the_real_daemon() {
    let (url, fx) = start().await;
    let c = Arc::new(FleetClient::new(&url));
    let names: Vec<String> = blocking!(c, c.repos())
        .unwrap()
        .into_iter()
        .map(|r| r.name)
        .collect();
    assert_eq!(names, ["core"]);
    assert_eq!(blocking!(c, c.ui_schema()).unwrap(), default_ui_schema());

    let agent: Agent = blocking!(c, c.spawn(&["core".to_string()], "fix it"))
        .unwrap()
        .agent;
    assert_eq!(blocking!(c, c.snapshot()).unwrap().agents.len(), 1);
    assert!(pane_contains(&fx, &agent.id, "ready").await);

    let (a, ans) = (agent.clone(), action("answer"));
    blocking!(c, c.run(&ans, &a, Some("$(x) \"q\" {a}"))).unwrap();
    assert!(
        pane_contains(&fx, &agent.id, "{a}").await,
        "answer text did not reach the agent"
    );

    let (a, pause, resume) = (agent.clone(), action("pause"), action("resume"));
    blocking!(c, c.run(&pause, &a, None)).unwrap();
    assert!(common::agent(&fx, &agent.id).await.paused);
    blocking!(c, c.run(&resume, &a, None)).unwrap();
    assert!(!common::agent(&fx, &agent.id).await.paused);

    std::fs::write(
        PathBuf::from(&agent.workspaces[0].worktree).join("new.txt"),
        "hello\n",
    )
    .unwrap();
    let id = agent.id.0.clone();
    let report = blocking!(c, c.diff(&id)).unwrap();
    assert_eq!(report.workspaces[0].repo, "core");
    assert!(report.workspaces[0].diff.contains("+hello"));

    let (a, stop) = (agent.clone(), action("stop"));
    blocking!(c, c.run(&stop, &a, None)).unwrap();
    assert_eq!(
        blocking!(c, c.snapshot()).unwrap().agents[0].status,
        fleet_api::Status::Stopped
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn errors_carry_the_daemons_message() {
    let (url, fx) = start().await;
    let c = Arc::new(FleetClient::new(&url));
    let err = blocking!(c, c.spawn(&["nope".to_string()], "x")).unwrap_err();
    assert!(err.0.contains("nope"), "{err}");
    let ghost = common::agent_fixture(&fx);
    let stop = action("stop");
    let err = blocking!(c, c.run(&stop, &ghost, None)).unwrap_err();
    assert!(err.0.contains("no such agent"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreachable_daemon_is_an_error_not_a_panic_or_a_hang() {
    let c = Arc::new(FleetClient::new("http://127.0.0.1:1"));
    let started = std::time::Instant::now();
    assert!(blocking!(c, c.snapshot()).is_err());
    assert!(started.elapsed() < Duration::from_secs(12));
}

#[tokio::test(flavor = "multi_thread")]
async fn event_stream_sends_connected_then_snapshot_then_events() {
    let (url, _fx) = start().await;
    let (tx, rx) = mpsc::channel();
    let _h = spawn_event_stream(&url, tx, wake());
    let next = || {
        rx.recv_timeout(Duration::from_secs(10))
            .expect("no message in 10s")
    };
    assert!(matches!(next(), StreamMsg::Connected));
    assert!(matches!(next(), StreamMsg::Snapshot(s) if s.seq == 0));
    let c = Arc::new(FleetClient::new(&url));
    let agent = blocking!(c, c.spawn(&["core".to_string()], "x"))
        .unwrap()
        .agent;
    assert!(
        matches!(next(), StreamMsg::Event(e) if matches!(&e.kind, fleet_api::EventKind::AgentCreated { agent: a } if a.id == agent.id))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn pane_stream_shows_the_agents_terminal() {
    let (url, _fx) = start().await;
    let c = Arc::new(FleetClient::new(&url));
    let agent = blocking!(c, c.spawn(&["core".to_string()], "x"))
        .unwrap()
        .agent;
    let (tx, rx) = mpsc::channel();
    let _h = spawn_pane_stream(&url, &agent.id.0, tx, wake());
    let mut seen = false;
    for _ in 0..20 {
        if let Ok(PaneMsg::Text(t)) = rx.recv_timeout(Duration::from_secs(10)) {
            if t.contains("ready") {
                seen = true;
                break;
            }
        }
    }
    assert!(seen, "pane text never arrived");
}

#[allow(dead_code)]
fn _unused(_: AgentId) {}

#[tokio::test(flavor = "multi_thread")]
async fn client_spawns_a_project_with_no_repos() {
    let (url, _fx) = start().await;
    let c = Arc::new(FleetClient::new(&url));
    let agent = blocking!(c, c.spawn(&[], "think first")).unwrap().agent;
    assert!(agent.workspaces.is_empty());
    assert_eq!(blocking!(c, c.snapshot()).unwrap().agents.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn client_adds_and_lists_repos() {
    let (url, fx) = start().await;
    let c = Arc::new(FleetClient::new(&url));
    let agent = blocking!(c, c.spawn(&[], "x")).unwrap().agent;
    let tools = make_repo(&fx._tmp.path().join("tools"));
    let path = tools.to_str().unwrap().to_string();
    let id = agent.id.0.clone();
    let ws = blocking!(c, c.add_repo(&id, &path)).unwrap();
    assert_eq!(ws.repo, "tools");
    let names: Vec<String> = blocking!(c, c.repos())
        .unwrap()
        .into_iter()
        .map(|r| r.name)
        .collect();
    assert!(
        names.contains(&"tools".to_string()) && names.contains(&"core".to_string()),
        "{names:?}"
    );
    let err = blocking!(c, c.add_repo("ghost", &path)).unwrap_err();
    assert!(err.0.contains("no such agent"), "{err}");
}

/// Holds an agent's `request_repo` call open in the background, as Claude Code does.
fn request_repo_held(
    url: &str,
    id: &str,
    token: &str,
    repo: &str,
) -> tokio::task::JoinHandle<Value> {
    let body = json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
        "params": {"name": "request_repo", "arguments": {"repo": repo, "why": "need it"}}});
    let addr: std::net::SocketAddr = url.trim_start_matches("http://").parse().unwrap();
    let (path, token) = (format!("/internal/mcp/{id}"), token.to_string());
    tokio::spawn(async move {
        let (status, text) = http(
            addr,
            "POST",
            &path,
            &[("X-Fleet-Token", &token)],
            Some(&body),
        )
        .await;
        assert_eq!(status, 200, "{text}");
        serde_json::from_str(&text).unwrap()
    })
}

async fn wait_pending(fx: &Fx, id: &AgentId) {
    assert!(
        wait_until(|| async { common::agent(fx, id).await.repo_request.is_some() }).await,
        "request never became pending"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn client_grant_and_refuse_round_trip() {
    let (url, fx) = start().await;
    let c = Arc::new(FleetClient::new(&url));
    let agent = blocking!(c, c.spawn(&[], "x")).unwrap().agent;
    let token = fx.store.agent_token(&agent.id).await.unwrap().unwrap();
    let tools = make_repo(&fx._tmp.path().join("tools"));
    let path = tools.to_str().unwrap().to_string();
    let id = agent.id.0.clone();

    let held = request_repo_held(&url, &id, &token, &path);
    wait_pending(&fx, &agent.id).await;
    let ws = blocking!(c, c.grant_repo(&id, &path)).unwrap();
    assert_eq!(ws.repo, "tools");
    let reply = held.await.unwrap();
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    // Nothing pending any more.
    assert!(blocking!(c, c.grant_repo(&id, &path)).is_err());

    let other = make_repo(&fx._tmp.path().join("other"));
    let other = other.to_str().unwrap().to_string();
    let held = request_repo_held(&url, &id, &token, &other);
    wait_pending(&fx, &agent.id).await;
    blocking!(c, c.refuse_repo(&id, "not this one")).unwrap();
    let reply = held.await.unwrap();
    assert!(
        reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("not this one"),
        "{reply}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn client_stop_and_the_stop_action_both_stop_the_agent() {
    let (url, _fx) = start().await;
    let c = Arc::new(FleetClient::new(&url));
    let core = ["core".to_string()];
    let first = blocking!(c, c.spawn(&core, "x")).unwrap().agent;
    let (a, stop) = (first.clone(), action("stop"));
    assert_eq!(blocking!(c, c.run(&stop, &a, None)).unwrap(), None);

    let second = blocking!(c, c.spawn(&core, "y")).unwrap().agent;
    let id = second.id.0.clone();
    blocking!(c, c.stop(&id)).unwrap();
    let agents = blocking!(c, c.snapshot()).unwrap().agents;
    assert_eq!(agents.len(), 2, "stopping keeps the card");
    assert!(agents
        .iter()
        .all(|a| a.status == fleet_api::Status::Stopped));
}
