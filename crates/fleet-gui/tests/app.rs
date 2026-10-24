use eframe::egui::{self, accesskit::Role, Vec2};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use fleet_api::{default_ui_schema, Agent, AgentId, BlockedReason, DiffReport, Event, EventKind, Snapshot, Status, Workspace};
use fleet_client::http::FleetClient;
use fleet_client::stream::{PaneMsg, StreamMsg};
use fleet_gui::app::{AppResult, FleetApp};
use std::time::{Duration, Instant};

const NOW: i64 = 1300;

fn ws(repo: &str) -> Workspace {
    Workspace { repo: repo.into(), branch: "fleet/x".into(), worktree: format!("/w/{repo}"), base: "abc".into(), has_changes: false }
}

fn agent(id: &str, task: &str, status: Status, repos: &[&str]) -> Agent {
    Agent {
        id: AgentId(id.into()),
        task: task.into(),
        workdir: "/w".into(),
        workspaces: repos.iter().map(|r| ws(r)).collect(),
        tmux_session: format!("fleet-{id}"),
        status,
        activity: "running Bash".into(),
        status_since: 1000,
        has_changes: false,
        paused: false,
        owner: "antonio".into(),
        blocked_reason: None,
    }
}

/// An app that starts no threads: tests feed it the messages the background threads would send.
fn app() -> FleetApp {
    let mut app = FleetApp::detached(FleetClient::new("http://127.0.0.1:1"), egui::Context::default());
    app.set_clock(Box::new(|| NOW));
    app.handle_result(AppResult::Schema(Ok(default_ui_schema())));
    app.handle_result(AppResult::Repos(Ok(vec!["core".into(), "portal".into()])));
    app
}

fn snapshot(agents: Vec<Agent>) -> StreamMsg {
    StreamMsg::Snapshot(Snapshot { seq: 1, agents })
}

fn harness(app: FleetApp) -> Harness<'static, FleetApp> {
    Harness::builder().with_size(Vec2::new(1400.0, 800.0)).build_ui_state(|ui, app: &mut FleetApp| app.ui(ui), app)
}

fn x(h: &Harness<'_, FleetApp>, label: &str) -> f32 {
    h.get_by_label(label).rect().center().x
}

fn status_changed(seq: u64, id: &str, status: Status) -> StreamMsg {
    StreamMsg::Event(Event {
        seq,
        kind: EventKind::StatusChanged {
            id: AgentId(id.into()),
            status,
            activity: "wants to run Bash".into(),
            since: 1200,
            has_changes: false,
            paused: false,
            blocked_reason: Some(BlockedReason::Permission),
        },
    })
}

fn wait_for(h: &mut Harness<'_, FleetApp>, mut done: impl FnMut(&FleetApp) -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        h.run();
        if done(h.state()) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn a_card_moves_when_a_status_event_arrives() {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent("a1", "fix the tests", Status::Working, &["core"])]));
    let mut h = harness(a);
    h.run();
    let working = x(&h, "Working");
    assert!((x(&h, "fix the tests") - working).abs() < 200.0);
    h.state_mut().handle_stream(status_changed(2, "a1", Status::NeedsYou));
    h.run();
    let needs = x(&h, "Needs you");
    assert!((x(&h, "fix the tests") - needs).abs() < 200.0, "the card did not follow its status");
    h.get_by_label("permission");
}

#[test]
fn a_snapshot_replaces_the_board() {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent("a1", "first", Status::Working, &["core"])]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("first");
    h.state_mut().handle_stream(StreamMsg::Snapshot(Snapshot { seq: 9, agents: vec![agent("b", "second", Status::Review, &["core"])] }));
    h.run();
    h.get_by_label("second");
    assert!(h.query_by_label("first").is_none());
}

#[test]
fn repo_filter_shows_agents_touching_any_selected_repo() {
    let mut a = app();
    a.handle_stream(snapshot(vec![
        agent("a", "core task", Status::Working, &["core"]),
        agent("b", "portal task", Status::Working, &["portal"]),
        agent("c", "both task", Status::Working, &["core", "portal"]),
    ]));
    let mut h = harness(a);
    h.run();
    h.get_by_role_and_label(Role::Button, "portal").click();
    h.run();
    assert!(h.query_by_label("core task").is_none(), "an agent outside the selected repo is shown");
    h.get_by_label("portal task");
    h.get_by_label("both task");
    h.get_by_role_and_label(Role::Button, "portal").click();
    h.run();
    h.get_by_label("core task");
}

#[test]
fn connection_indicator_follows_connected_and_disconnected() {
    let mut h = harness(app());
    h.run();
    h.get_by_label_contains("reconnecting");
    h.state_mut().handle_stream(StreamMsg::Connected);
    h.run();
    h.get_by_label_contains("live");
    h.state_mut().handle_stream(StreamMsg::Disconnected);
    h.run();
    h.get_by_label_contains("reconnecting");
}

#[test]
fn an_action_failure_is_shown_instead_of_swallowed() {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent("a1", "work", Status::Working, &["core"])]));
    let mut h = harness(a);
    h.run();
    h.state_mut().handle_result(AppResult::Action(Err("agent is not running".into())));
    h.run();
    h.get_by_label("agent is not running");
}

#[test]
fn running_an_action_goes_through_the_client_on_a_background_thread() {
    // The client points at a closed port, so the call fails; the failure must come back as a message.
    let mut a = app();
    a.handle_stream(snapshot(vec![agent("a1", "work", Status::Working, &["core"])]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("Pause").click();
    h.run();
    assert!(wait_for(&mut h, |app| app.problem().is_some()), "no result came back from the background call");
}

#[test]
fn clicking_a_card_opens_a_read_only_preview_with_a_diff_tab() {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent("a1", "open me", Status::Working, &["core"])]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("open me").click();
    h.run();
    assert_eq!(h.state().preview_agent(), Some("a1"));
    h.get_by_label("Terminal");
    h.get_by_label("Diff");
    assert_eq!(h.query_all_by_role(Role::TextInput).count() + h.query_all_by_role(Role::MultilineTextInput).count(), 1, "only the spawn prompt may be typeable");
    h.state_mut().handle_pane(PaneMsg::Text("$ ready".into()));
    h.run();
    h.get_by_label_contains("$ ready");
}

#[test]
fn the_diff_tab_loads_through_the_app_and_shows_the_result() {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent("a1", "open me", Status::Working, &["core"])]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("open me").click();
    h.run();
    h.get_by_label("Diff").click();
    h.run();
    h.state_mut().handle_result(AppResult::Diff(
        "a1".into(),
        Ok(DiffReport { workspaces: vec![fleet_api::WorkspaceDiff { repo: "core".into(), diff: "+hello\n".into(), truncated: false }] }),
    ));
    h.run();
    h.get_by_label_contains("+hello");
}

#[test]
fn the_preview_closes_when_its_agent_is_removed() {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent("a1", "open me", Status::Working, &["core"])]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("open me").click();
    h.run();
    assert!(h.state().preview_agent().is_some());
    h.state_mut().handle_stream(StreamMsg::Event(Event { seq: 2, kind: EventKind::AgentRemoved { id: AgentId("a1".into()) } }));
    h.run();
    assert_eq!(h.state().preview_agent(), None);
}

#[test]
fn closing_the_preview_drops_it() {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent("a1", "open me", Status::Working, &["core"])]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("open me").click();
    h.run();
    h.get_by_label("Close").click();
    h.run();
    assert_eq!(h.state().preview_agent(), None);
}

#[test]
fn spawn_success_clears_the_prompt_and_failure_keeps_it_with_the_error() {
    // A listener that accepts connections and never answers keeps the spawn call in flight for the whole test,
    // unlike a closed port, which would fail (and clear "busy") before the assertions look.
    let hold = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut a = FleetApp::detached(FleetClient::new(&format!("http://{}", hold.local_addr().unwrap())), egui::Context::default());
    a.handle_result(AppResult::Schema(Ok(default_ui_schema())));
    a.handle_result(AppResult::Repos(Ok(vec!["core".into(), "portal".into()])));
    let mut h = harness(a);
    h.run();
    let prompt = h.get_by_role(Role::MultilineTextInput);
    prompt.click();
    prompt.type_text("do the thing");
    h.run();
    h.get_by_role_and_label(Role::Button, "Start agent").click();
    h.run();
    assert!(h.state().spawn_state().busy, "a spawn is in flight");
    h.state_mut().handle_result(AppResult::Spawn(Err("unknown repo \"x\"".into())));
    h.run();
    h.get_by_label("unknown repo \"x\"");
    assert_eq!(h.state().spawn_state().prompt, "do the thing");
    assert!(!h.state().spawn_state().busy);
    h.state_mut().handle_result(AppResult::Spawn(Ok(agent("n", "new", Status::Queued, &["core"]))));
    h.run();
    assert_eq!(h.state().spawn_state().prompt, "");
    assert_eq!(h.state().spawn_state().error, None);
}

#[test]
fn schema_load_failure_shows_the_error_and_a_retry() {
    let mut a = FleetApp::detached(FleetClient::new("http://127.0.0.1:1"), egui::Context::default());
    a.handle_result(AppResult::Schema(Err("connection refused".into())));
    let mut h = harness(a);
    h.run();
    h.get_by_label_contains("connection refused");
    h.get_by_label("Retry").click();
    h.run();
    assert!(h.query_by_label_contains("connection refused").is_none(), "retry clears the old error while it loads");
}

#[test]
fn the_ui_does_not_block_while_the_daemon_is_unreachable() {
    // The real constructor starts the background threads against a port nothing listens on.
    let mut h = harness(FleetApp::new(FleetClient::new("http://127.0.0.1:1"), egui::Context::default()));
    let t = Instant::now();
    for _ in 0..10 {
        h.step();
    }
    assert!(t.elapsed() < Duration::from_millis(500), "ten frames took {:?}", t.elapsed());
    assert!(wait_for(&mut h, |app| app.problem().is_some() || app.load_error().is_some()), "the failed load was never reported");
}
