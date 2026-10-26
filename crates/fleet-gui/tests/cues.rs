use eframe::egui::{self, accesskit::Role, Vec2};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use fleet_api::{default_ui_schema, Agent, AgentId, Snapshot, Status};
use fleet_client::http::FleetClient;
use fleet_client::stream::StreamMsg;
use fleet_gui::app::{AppResult, FleetApp};
use fleet_gui::components::PreviewTab;
use fleet_gui::cues::{parse_cue, Cue, CueFile};
use fleet_gui::theme;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

fn agent(id: &str, goal: &str) -> Agent {
    Agent {
        id: AgentId(id.into()),
        goal: goal.into(),
        workdir: "/w".into(),
        workspaces: vec![],
        tmux_session: format!("fleet-{id}"),
        status: Status::Working,
        activity: "running Bash".into(),
        status_since: 1000,
        has_changes: false,
        paused: false,
        owner: "antonio".into(),
        blocked_reason: None,
        repo_request: None,
        stop_cause: None,
        disk_bytes: None,
        persona: "p".into(),
        product: "p".into(),
        kind: fleet_api::AgentKind::Solo,
        task: None,
        role: None,
        step: None,
        attach: None,
    }
}

fn snapshot(agents: Vec<Agent>) -> StreamMsg {
    StreamMsg::Snapshot(Snapshot { seq: 1, agents })
}

/// An app on `base` with agent `a1` on the board.
fn app_on(base: &str) -> FleetApp {
    let mut app = FleetApp::detached(FleetClient::new(base), egui::Context::default());
    app.set_clock(Box::new(|| 1300));
    app.handle_result(AppResult::Schema(Ok(default_ui_schema())));
    app.handle_stream(snapshot(vec![agent("a1", "open me")]));
    app
}

/// Its calls fail against a closed port, which is all the panel tests need.
fn app() -> FleetApp {
    app_on("http://127.0.0.1:1")
}

fn harness(app: FleetApp) -> Harness<'static, FleetApp> {
    Harness::builder()
        .with_size(Vec2::new(1400.0, 800.0))
        .build_ui_state(|ui, app: &mut FleetApp| app.ui(ui), app)
}

fn append(path: &Path, text: &str) {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap()
        .write_all(text.as_bytes())
        .unwrap();
}

fn open(agent: &str, tab: PreviewTab) -> Cue {
    Cue::Open {
        agent: agent.into(),
        tab,
    }
}

#[test]
fn parse_cue_accepts_open_and_close_only() {
    assert_eq!(
        parse_cue("open a1 terminal"),
        Some(open("a1", PreviewTab::Terminal))
    );
    assert_eq!(
        parse_cue("open a1 diff"),
        Some(open("a1", PreviewTab::Diff))
    );
    assert_eq!(parse_cue("close"), Some(Cue::Close));
    assert_eq!(parse_cue("  close \n"), Some(Cue::Close));
    for bad in [
        "",
        "open",
        "open a1",
        "open a1 log",
        "open a1 diff extra",
        "close now",
        "select a1",
        "OPEN a1 diff",
    ] {
        assert_eq!(parse_cue(bad), None, "{bad:?}");
    }
}

#[test]
fn poll_returns_only_new_lines() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cues");
    let mut cues = CueFile::new(path.clone());
    assert_eq!(cues.poll(), vec![], "a missing file yields nothing");

    append(&path, "open a1 terminal\n");
    assert_eq!(cues.poll(), vec![open("a1", PreviewTab::Terminal)]);
    assert_eq!(cues.poll(), vec![]);

    // A line still being written waits for its newline.
    append(&path, "bogus\nopen a1 diff\nclo");
    assert_eq!(cues.poll(), vec![open("a1", PreviewTab::Diff)]);
    append(&path, "se\n");
    assert_eq!(cues.poll(), vec![Cue::Close]);
}

#[test]
fn open_cue_opens_the_panel_like_a_click() {
    let mut h = harness(app());
    h.run();
    h.state_mut().apply_cue(open("a1", PreviewTab::Terminal));
    h.run();
    assert_eq!(h.state().preview_agent(), Some("a1"));
    h.get_by_role_and_label(Role::Button, "Terminal");
    // Refresh is drawn on the Diff tab only.
    assert!(h.query_by_label("Refresh").is_none());
}

/// The first `GET` for `a1`'s diff that reaches `hold`; other calls (the pane stream) are accepted and held.
fn diff_requested(hold: &std::net::TcpListener) -> bool {
    use std::io::{BufRead, BufReader};
    hold.set_nonblocking(true).unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    let mut held = Vec::new();
    while Instant::now() < end {
        let Ok((stream, _)) = hold.accept() else {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        };
        stream.set_nonblocking(false).unwrap();
        let mut first = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut first)
            .unwrap();
        if first.starts_with("GET /api/agents/a1/diff ") {
            return true;
        }
        held.push(stream);
    }
    false
}

#[test]
fn open_diff_cue_loads_the_diff() {
    let hold = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut h = harness(app_on(&format!("http://{}", hold.local_addr().unwrap())));
    h.run();
    h.state_mut().apply_cue(open("a1", PreviewTab::Diff));
    h.run();
    assert_eq!(h.state().preview_agent(), Some("a1"));
    h.get_by_label("Refresh");
    assert!(diff_requested(&hold), "no diff call went out");
}

#[test]
fn close_cue_closes_the_panel() {
    let mut h = harness(app());
    h.run();
    h.state_mut().apply_cue(open("a1", PreviewTab::Terminal));
    h.run();
    assert!(h.state().preview_agent().is_some());
    h.state_mut().apply_cue(Cue::Close);
    h.run();
    assert_eq!(h.state().preview_agent(), None);
}

#[test]
fn open_cue_for_unknown_agent_changes_nothing() {
    let mut a = app();
    a.set_view(fleet_gui::views::View::Settings);
    let mut h = harness(a);
    h.run();
    h.state_mut().apply_cue(open("nobody", PreviewTab::Diff));
    h.run();
    assert_eq!(h.state().preview_agent(), None);
    assert_eq!(h.state().view(), fleet_gui::views::View::Settings);
}

#[test]
fn cues_in_the_file_reach_the_app_each_frame() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cues");
    let mut h = harness(app().with_cues(CueFile::new(path.clone())));
    h.run();
    append(&path, "open a1 terminal\n");
    h.run();
    assert_eq!(h.state().preview_agent(), Some("a1"));
    append(&path, "close\n");
    h.run();
    assert_eq!(h.state().preview_agent(), None);
}

#[test]
fn cues_force_animation_without_focus() {
    let mut plain = harness(app());
    plain.step();
    assert!(!theme::animate(&plain.ctx));

    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(app().with_cues(CueFile::new(dir.path().join("cues"))));
    h.step();
    assert!(theme::animate(&h.ctx), "the viewport never reports focus");
    assert_eq!(h.ctx.global_style().animation_time, theme::ANIMATION_TIME);
}
