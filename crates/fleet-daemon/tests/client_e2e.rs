//! fleet-client against the real daemon router (the daemon crate owns this test so the dependency points one way).
mod common;

use common::*;
use fleet_api::{default_ui_schema, ActionDef, Agent, AgentId};
use fleet_client::http::FleetClient;
use fleet_client::stream::{spawn_event_stream, spawn_pane_stream, PaneMsg, StreamMsg};
use fleet_daemon::http::{bind_loopback, router, AppState};
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::time::Duration;

async fn start() -> (String, Fx) {
    let fx = fixture().await;
    let listener = bind_loopback("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = AppState::new(fx.svc.clone(), fx.store.clone(), fx.tmux.clone(), addr.port());
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
    default_ui_schema().actions.into_iter().find(|a| a.id == id).unwrap()
}

fn wake() -> Arc<dyn Fn() + Send + Sync> {
    Arc::new(|| {})
}

#[tokio::test(flavor = "multi_thread")]
async fn client_round_trips_against_the_real_daemon() {
    let (url, fx) = start().await;
    let c = Arc::new(FleetClient::new(&url));
    assert_eq!(blocking!(c, c.repos()).unwrap(), ["core"]);
    assert_eq!(blocking!(c, c.ui_schema()).unwrap(), default_ui_schema());

    let agent: Agent = blocking!(c, c.spawn("core", "fix it")).unwrap();
    assert_eq!(blocking!(c, c.snapshot()).unwrap().agents.len(), 1);
    assert!(pane_contains(&fx, &agent.id, "ready").await);

    let (a, ans) = (agent.clone(), action("answer"));
    blocking!(c, c.run(&ans, &a, Some("$(x) \"q\" {a}"))).unwrap();
    assert!(pane_contains(&fx, &agent.id, "{a}").await, "answer text did not reach the agent");

    let (a, pause, resume) = (agent.clone(), action("pause"), action("resume"));
    blocking!(c, c.run(&pause, &a, None)).unwrap();
    assert!(common::agent(&fx, &agent.id).await.paused);
    blocking!(c, c.run(&resume, &a, None)).unwrap();
    assert!(!common::agent(&fx, &agent.id).await.paused);

    std::fs::write(PathBuf::from(&agent.workspaces[0].worktree).join("new.txt"), "hello\n").unwrap();
    let id = agent.id.0.clone();
    let report = blocking!(c, c.diff(&id)).unwrap();
    assert_eq!(report.workspaces[0].repo, "core");
    assert!(report.workspaces[0].diff.contains("+hello"));

    let (a, kill) = (agent.clone(), action("kill"));
    blocking!(c, c.run(&kill, &a, None)).unwrap();
    assert!(blocking!(c, c.snapshot()).unwrap().agents.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn errors_carry_the_daemons_message() {
    let (url, fx) = start().await;
    let c = Arc::new(FleetClient::new(&url));
    let err = blocking!(c, c.spawn("nope", "x")).unwrap_err();
    assert!(err.0.contains("nope"), "{err}");
    let ghost = common::agent_fixture(&fx);
    let kill = action("kill");
    let err = blocking!(c, c.run(&kill, &ghost, None)).unwrap_err();
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
    let next = || rx.recv_timeout(Duration::from_secs(10)).expect("no message in 10s");
    assert!(matches!(next(), StreamMsg::Connected));
    assert!(matches!(next(), StreamMsg::Snapshot(s) if s.seq == 0));
    let c = Arc::new(FleetClient::new(&url));
    let agent = blocking!(c, c.spawn("core", "x")).unwrap();
    assert!(matches!(next(), StreamMsg::Event(e) if matches!(&e.kind, fleet_api::EventKind::AgentCreated { agent: a } if a.id == agent.id)));
}

#[tokio::test(flavor = "multi_thread")]
async fn pane_stream_shows_the_agents_terminal() {
    let (url, _fx) = start().await;
    let c = Arc::new(FleetClient::new(&url));
    let agent = blocking!(c, c.spawn("core", "x")).unwrap();
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
