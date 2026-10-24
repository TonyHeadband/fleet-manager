use eframe::egui::{self, accesskit::Role, Key, Modifiers, Vec2};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use fleet_api::{default_ui_schema, Agent, AgentId, BlockedReason, DiffReport, Event, EventKind, Snapshot, Status, Workspace};
use fleet_client::config::{GuiSettings, ThemeChoice};
use fleet_client::http::FleetClient;
use fleet_client::stream::{PaneMsg, StreamMsg};
use fleet_gui::app::{AppResult, FleetApp};
use fleet_gui::theme;
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

/// For tests that open the New agent dialog: a modal needs more frames than the default to settle.
fn modal_harness(app: FleetApp) -> Harness<'static, FleetApp> {
    Harness::builder().with_max_steps(60).with_size(Vec2::new(1400.0, 800.0)).build_ui_state(|ui, app: &mut FleetApp| app.ui(ui), app)
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
    // By name: the New agent dialog has a combo box too.
    h.get_by_role_and_label(Role::ComboBox, "Repo filter").click();
    h.run();
    h.get_by_role_and_label(Role::CheckBox, "portal").click();
    h.run();
    assert!(h.query_by_label("core task").is_none(), "an agent outside the selected repo is shown");
    h.get_by_label("portal task");
    h.get_by_label("both task");
    h.get_by_role_and_label(Role::CheckBox, "portal").click();
    h.run();
    h.get_by_label("core task");
}

#[test]
fn theme_switch_cycles_and_saves_the_choice() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("gui.toml");
    let ctx = egui::Context::default();
    let mut a = FleetApp::detached(FleetClient::new("http://127.0.0.1:1"), ctx.clone()).with_settings(path.clone());
    a.handle_result(AppResult::Schema(Ok(default_ui_schema())));
    let mut h = harness(a);
    h.run();
    h.get_by_role_and_label(Role::Button, "Theme: System").click();
    h.run();
    assert_eq!(h.state().theme_choice(), ThemeChoice::Light);
    assert_eq!(ctx.options(|o| o.theme_preference), egui::ThemePreference::Light);
    assert_eq!(GuiSettings::load(&path).theme, ThemeChoice::Light);
    h.get_by_role_and_label(Role::Button, "Theme: Light");
}

#[test]
fn action_error_shows_under_the_top_bar() {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent("a1", "work", Status::Working, &["core"])]));
    let mut h = harness(a);
    h.run();
    h.state_mut().handle_result(AppResult::Action(Err("agent is not running".into())));
    h.run();
    let y = |label: &str| h.get_by_label(label).rect().center().y;
    let error = y("agent is not running");
    assert!(error > y("fleet-board"), "the error is above the top bar's content");
    assert!(error < y("Queued"), "the error is below the board's column headers");
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
    assert_eq!(h.query_all_by_role(Role::TextInput).count() + h.query_all_by_role(Role::MultilineTextInput).count(), 0, "the preview must not add a typeable field");
    h.state_mut().handle_pane(PaneMsg::Text("$ ready".into()));
    h.run();
    h.get_by_label_contains("$ ready");
}

#[test]
fn opening_a_needs_you_card_focuses_the_reply_box() {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent("a1", "ask me", Status::NeedsYou, &["core"])]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("ask me").click();
    h.run();
    assert!(h.get_by_role(Role::MultilineTextInput).is_focused());
    h.get_by_role_and_label(Role::Button, "Send");
}

#[test]
fn a_reply_is_kept_per_agent_and_cleared_when_sent() {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent("a", "first", Status::NeedsYou, &["core"]), agent("b", "second", Status::NeedsYou, &["core"])]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("first").click();
    h.run();
    h.get_by_role(Role::MultilineTextInput).type_text("draft a");
    h.run();
    h.get_by_label("second").click();
    h.run();
    h.get_by_role(Role::MultilineTextInput).type_text("draft b");
    h.run();
    h.get_by_label("first").click();
    h.run();
    assert_eq!(h.get_by_role(Role::MultilineTextInput).value().unwrap_or_default(), "draft a");
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run();
    assert_eq!(h.state().reply_text("a"), "");
    assert_eq!(h.state().reply_text("b"), "draft b");
}

#[test]
fn a_working_agent_has_no_reply_box() {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent("a1", "busy", Status::Working, &["core"])]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("busy").click();
    h.run();
    assert_eq!(h.query_all_by_role(Role::MultilineTextInput).count(), 0);
}

#[test]
fn a_reply_box_that_appears_later_does_not_take_the_focus() {
    // Opened while Working (no box), the agent then turns Needs you: the cursor may be in the New agent dialog by then.
    let mut a = app();
    a.handle_stream(snapshot(vec![agent("a1", "busy", Status::Working, &["core"])]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("busy").click();
    h.run();
    h.state_mut().handle_stream(status_changed(2, "a1", Status::NeedsYou));
    h.run();
    assert!(!h.get_by_role(Role::MultilineTextInput).is_focused());
}

#[test]
fn undo_in_one_agents_reply_box_cannot_bring_back_another_agents_draft() {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent("a", "first", Status::NeedsYou, &["core"]), agent("b", "second", Status::NeedsYou, &["core"])]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("first").click();
    h.run();
    h.get_by_role(Role::MultilineTextInput).type_text("secret");
    h.run();
    // Past the undoer's 1 s stable time, so the draft is a point it can return to. kittest's clock only moves by
    // `predicted_dt` per frame, never with wall time, so it is moved forward explicitly.
    let now = h.ctx.input(|i| i.time);
    h.input_mut().time = Some(now + 2.0);
    h.step();
    h.step();
    h.get_by_label("second").click();
    h.run();
    assert!(h.get_by_role(Role::MultilineTextInput).is_focused());
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(h.get_by_role(Role::MultilineTextInput).value().unwrap_or_default(), "");
    assert_eq!(h.state().reply_text("b"), "");
    assert_eq!(h.state().reply_text("a"), "secret");
}

#[test]
fn opening_a_card_fills_the_panel_path() {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent("a1", "open me", Status::Working, &["core"])]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("open me").click();
    h.run();
    h.get_by_label("core / fleet/x");
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

/// An app whose spawn call hangs: a listener that accepts connections and never answers keeps it in flight for
/// the whole test, unlike a closed port, which would fail (and clear "busy") before the assertions look.
fn hanging_app() -> (FleetApp, std::net::TcpListener) {
    let hold = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut a = FleetApp::detached(FleetClient::new(&format!("http://{}", hold.local_addr().unwrap())), egui::Context::default());
    a.handle_result(AppResult::Schema(Ok(default_ui_schema())));
    a.handle_result(AppResult::Repos(Ok(vec!["core".into(), "portal".into()])));
    (a, hold)
}

/// Opens the dialog, types a prompt and starts the agent, leaving the spawn in flight.
fn start_an_agent(h: &mut Harness<'static, FleetApp>) {
    h.run();
    h.get_by_role_and_label(Role::Button, "New agent").click();
    h.run();
    let prompt = h.get_by_role(Role::MultilineTextInput);
    prompt.click();
    prompt.type_text("do the thing");
    h.run();
    h.get_by_role_and_label(Role::Button, "Start agent").click();
    h.run();
    assert!(h.state().spawn_state().busy, "a spawn is in flight");
}

#[test]
fn spawn_success_clears_the_prompt_and_failure_keeps_it_with_the_error() {
    let (a, _hold) = hanging_app();
    let mut h = modal_harness(a);
    start_an_agent(&mut h);
    h.state_mut().handle_result(AppResult::Spawn(Err("unknown repo \"x\"".into())));
    h.run();
    h.get_by_label("unknown repo \"x\"");
    assert_eq!(h.state().spawn_state().prompt, "do the thing");
    assert!(!h.state().spawn_state().busy);
    assert!(h.state().spawn_state().open, "a failure keeps the dialog open");
    assert_eq!(h.state().problem(), None, "the dialog is showing the error already");
    h.state_mut().handle_result(AppResult::Spawn(Ok(agent("n", "new", Status::Queued, &["core"]))));
    h.run();
    assert_eq!(h.state().spawn_state().prompt, "");
    assert_eq!(h.state().spawn_state().error, None);
    assert!(!h.state().spawn_state().open);
}

#[test]
fn spawn_failure_after_closing_the_dialog_shows_in_the_problem_line() {
    let (a, _hold) = hanging_app();
    let mut h = modal_harness(a);
    start_an_agent(&mut h);
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(!h.state().spawn_state().open);
    h.state_mut().handle_result(AppResult::Spawn(Err("boom".into())));
    h.run();
    assert_eq!(h.state().problem(), Some("Could not start the agent: boom"));
    assert!(!h.state().spawn_state().open, "a failure must not reopen the dialog");
    assert_eq!(h.state().spawn_state().error.as_deref(), Some("boom"));
    h.get_by_label("Could not start the agent: boom");
}

#[test]
fn spawn_success_after_closing_does_not_reopen() {
    let (a, _hold) = hanging_app();
    let mut h = modal_harness(a);
    start_an_agent(&mut h);
    h.key_press(egui::Key::Escape);
    h.run();
    h.state_mut().handle_result(AppResult::Spawn(Ok(agent("n", "new", Status::Queued, &["core"]))));
    h.run();
    assert!(!h.state().spawn_state().open);
    assert_eq!(h.state().spawn_state().prompt, "");
}

#[test]
fn a_later_successful_spawn_clears_the_earlier_failure_line() {
    let (a, _hold) = hanging_app();
    let mut h = modal_harness(a);
    start_an_agent(&mut h);
    h.key_press(egui::Key::Escape);
    h.run();
    h.state_mut().handle_result(AppResult::Spawn(Err("boom".into())));
    h.run();
    assert_eq!(h.state().problem(), Some("Could not start the agent: boom"));
    // The user tries again from the reopened dialog, and this time it works.
    h.get_by_role_and_label(Role::Button, "New agent").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Start agent").click();
    h.run();
    h.state_mut().handle_result(AppResult::Spawn(Ok(agent("n", "new", Status::Queued, &["core"]))));
    h.run();
    assert_eq!(h.state().problem(), None, "the old failure must not outlive a success");
    assert!(h.query_by_label("Could not start the agent: boom").is_none());
}

#[test]
fn spawn_success_keeps_a_dialog_reopened_and_typed_into_while_in_flight() {
    let (a, _hold) = hanging_app();
    let mut h = modal_harness(a);
    start_an_agent(&mut h);
    h.key_press(egui::Key::Escape);
    h.run();
    h.get_by_role_and_label(Role::Button, "New agent").click();
    h.run();
    h.get_by_role(Role::MultilineTextInput).type_text(" and another");
    h.run();
    let typed = h.state().spawn_state().prompt.clone();
    assert_ne!(typed, "do the thing", "the reopened prompt was typed into");
    h.state_mut().handle_result(AppResult::Spawn(Ok(agent("n", "new", Status::Queued, &["core"]))));
    h.run();
    assert!(h.state().spawn_state().open, "the dialog the user is typing in must stay open");
    assert_eq!(h.state().spawn_state().prompt, typed, "the new text must be kept");
    assert!(!h.state().spawn_state().busy);
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

/// The colour a viewer sees at `p`: the last shape painted over it (filled rect or line), in paint order.
fn colour_at(h: &Harness<'_, FleetApp>, p: egui::Pos2) -> Option<egui::Color32> {
    fn walk(shape: &egui::Shape, p: egui::Pos2, top: &mut Option<egui::Color32>) {
        match shape {
            egui::Shape::Rect(r) if r.fill.a() > 0 && r.rect.contains(p) => *top = Some(r.fill),
            egui::Shape::LineSegment { points: [a, b], stroke } => {
                let near = egui::Rect::from_two_pos(*a, *b).expand(stroke.width / 2.0);
                if stroke.width > 0.0 && near.contains(p) {
                    *top = Some(stroke.color);
                }
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, p, top)),
            _ => {}
        }
    }
    let mut top = None;
    for c in &h.output().shapes {
        if c.clip_rect.contains(p) {
            walk(&c.shape, p, &mut top);
        }
    }
    top
}

/// The smallest filled rect of `fill` that holds `p`.
fn frame_around(h: &Harness<'_, FleetApp>, p: egui::Pos2, fill: egui::Color32) -> egui::Rect {
    fn walk(shape: &egui::Shape, out: &mut Vec<(egui::Rect, egui::Color32)>) {
        match shape {
            egui::Shape::Rect(r) => out.push((r.rect, r.fill)),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut all = Vec::new();
    h.output().shapes.iter().for_each(|c| walk(&c.shape, &mut all));
    all.into_iter()
        .filter(|(r, f)| *f == fill && r.contains(p))
        .map(|(r, _)| r)
        .min_by(|a, b| a.area().total_cmp(&b.area()))
        .expect("a frame of that fill around the point")
}

#[test]
fn column_headers_are_flush_and_bordered_and_dividers_run_full_height() {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent("a1", "work", Status::Working, &["core"])]));
    let mut h = harness(a);
    h.run();
    let t = theme::tokens(egui::Visuals::default().dark_mode);
    let centre = |label: &str| h.get_by_label(label).rect().center();
    // The harness hosts the app inside an 8 px outer margin, so the window's edges are the top bar's sides and
    // 8 px above the harness bottom.
    let top_bar = frame_around(&h, centre("fleet-board"), t.surface);
    let bottom = 800.0 - 8.0;
    let header = frame_around(&h, centre("Queued"), t.surface);
    let col_w = top_bar.width() / 5.0;

    assert!(header.left() - top_bar.left() <= 0.5 && header.width() >= col_w - 1.0, "header is not full column width: {header:?}");
    assert!((header.top() - top_bar.bottom()).abs() <= 1.0, "header floats under the top bar: {header:?} vs {top_bar:?}");
    // The whole last pixel row: a line centred on the edge is half covered by the body and reads as no line.
    for dy in [0.9, 0.1] {
        assert_eq!(colour_at(&h, egui::pos2(header.center().x, header.bottom() - dy)), Some(t.border), "no full rule under the header");
    }

    for i in 1..5 {
        let x = top_bar.left() + col_w * i as f32;
        for y in [top_bar.bottom() + 1.5, bottom - 1.0] {
            assert_eq!(colour_at(&h, egui::pos2(x, y)), Some(t.border), "divider {i} missing at y={y}");
        }
    }
}

#[test]
fn egui_transitions_follow_the_gate() {
    let mut h = harness(app());
    h.step();
    assert_eq!(h.ctx.global_style().animation_time, 0.0);

    h.input_mut().viewports.get_mut(&egui::ViewportId::ROOT).unwrap().focused = Some(true);
    h.step();
    assert_eq!(h.ctx.global_style().animation_time, theme::ANIMATION_TIME);

    h.input_mut().viewports.get_mut(&egui::ViewportId::ROOT).unwrap().focused = Some(false);
    h.step();
    assert_eq!(h.ctx.global_style().animation_time, 0.0);
}
