use std::path::PathBuf;
use eframe::egui::{self, accesskit::Role, Key, Modifiers, Vec2};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use fleet_api::{default_ui_schema, Agent, AgentId, BlockedReason, DiffReport, Event, EventKind, RepoRequest, Snapshot, Status, Workspace};
use fleet_client::config::{GuiSettings, ThemeChoice};
use fleet_client::http::FleetClient;
use fleet_client::stream::{PaneMsg, StreamMsg};
use fleet_gui::app::{AppResult, FleetApp};
use fleet_gui::theme;
use std::time::{Duration, Instant};

const NOW: i64 = 1300;

fn ws(repo: &str) -> Workspace {
    Workspace { repo: repo.into(), branch: "fleet/x".into(), worktree: format!("/w/{repo}"), base: "abc".into(), from_branch: None, has_changes: false, owned: false }
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
        repo_request: None,
    }
}

/// An app that starts no threads: tests feed it the messages the background threads would send.
fn app() -> FleetApp {
    let mut app = FleetApp::detached(FleetClient::new("http://127.0.0.1:1"), egui::Context::default());
    app.set_clock(Box::new(|| NOW));
    app.handle_result(AppResult::Schema(Ok(default_ui_schema())));
    app.handle_result(AppResult::Repos(Ok(vec![repo_info("core"), repo_info("portal")])));
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
            repo_request: None,
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
    h.get_by_label("Permission needed");
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
    // By name: the New agent dialog has a picker too.
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
    a.handle_result(AppResult::Repos(Ok(vec![repo_info("core"), repo_info("portal")])));
    (a, hold)
}

/// Opens the dialog, types a prompt and starts the agent, leaving the spawn in flight.
fn start_an_agent(h: &mut Harness<'static, FleetApp>) {
    h.run();
    h.get_by_role_and_label(Role::Button, "New project").click();
    h.run();
    let prompt = h.get_by_role(Role::MultilineTextInput);
    prompt.click();
    prompt.type_text("do the thing");
    h.run();
    h.get_by_role_and_label(Role::Button, "Start project").click();
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
fn a_successful_spawn_clears_the_chosen_repos() {
    let (a, _hold) = hanging_app();
    let mut h = modal_harness(a);
    h.run();
    h.get_by_role_and_label(Role::Button, "New project").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "core").click();
    let goal = h.get_by_role(Role::MultilineTextInput);
    goal.click();
    goal.type_text("go");
    h.run();
    h.get_by_role_and_label(Role::Button, "Start project").click();
    h.run();
    h.state_mut().handle_result(AppResult::Spawn(Ok(agent("n", "new", Status::Queued, &["core"]))));
    h.run();
    assert!(h.state().spawn_state().picker.chosen.is_empty());
}

#[test]
fn reopening_the_dialog_keeps_the_chosen_repos() {
    let (a, _hold) = hanging_app();
    let mut h = modal_harness(a);
    h.run();
    h.get_by_role_and_label(Role::Button, "New project").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "core").click();
    h.run();
    h.key_press(egui::Key::Escape);
    h.run();
    h.get_by_role_and_label(Role::Button, "New project").click();
    h.run();
    assert_eq!(h.state().spawn_state().picker.chosen, ["core"]);
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
    h.get_by_role_and_label(Role::Button, "New project").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Start project").click();
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
    h.get_by_role_and_label(Role::Button, "New project").click();
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
    let mut h = harness(FleetApp::new(FleetClient::new("http://127.0.0.1:1"), egui::Context::default(), PathBuf::new(), PathBuf::new(), PathBuf::new()));
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

fn repo_info(name: &str) -> fleet_api::RepoInfo {
    fleet_api::RepoInfo { name: name.into(), source: fleet_api::RepoSource::Path, location: format!("/repos/{name}") }
}

#[test]
fn a_kill_that_kept_the_folder_shows_on_the_problem_line_until_the_next_success() {
    let mut a = app();
    a.handle_result(AppResult::Action(Ok(Some("kept /p/x: it contains notes.txt".into()))));
    assert_eq!(a.problem(), Some("kept /p/x: it contains notes.txt"));
    a.handle_result(AppResult::Action(Ok(None)));
    assert_eq!(a.problem(), None);
}

fn waiting_for_portal(id: &str) -> Agent {
    let mut a = agent(id, "needs a repo", Status::NeedsYou, &["core"]);
    a.blocked_reason = Some(BlockedReason::RepoAccess);
    a.repo_request = Some(RepoRequest { repo: "portal".into(), why: "for the login form".into() });
    a
}

#[test]
fn a_pending_request_takes_the_panel_instead_of_the_reply_box() {
    let mut a = app();
    a.handle_stream(snapshot(vec![waiting_for_portal("a1")]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("needs a repo").click();
    h.run();
    h.get_by_label("Wants portal");
    assert!(h.query_by_role(Role::MultilineTextInput).is_none());
    assert!(h.get_by_role_and_label(Role::TextInput, "Grant access to").is_focused());
    h.get_by_label("Repo access");
}

#[test]
fn grant_and_refuse_go_through_the_client_and_failures_show() {
    for (button, who) in [("Grant", "grant"), ("Refuse", "refuse")] {
        let mut a = app();
        a.handle_stream(snapshot(vec![waiting_for_portal("a1")]));
        let mut h = harness(a);
        h.run();
        h.get_by_label("needs a repo").click();
        h.run();
        h.get_by_role_and_label(Role::Button, button).click();
        assert!(wait_for(&mut h, |a| a.problem().is_some()), "{who}: the closed port's failure never came back");
    }
}

#[test]
fn an_edited_grant_target_survives_a_failed_grant_and_is_kept_per_agent() {
    let mut a = app();
    a.handle_stream(snapshot(vec![waiting_for_portal("a1")]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("needs a repo").click();
    h.run();
    let field = h.get_by_role_and_label(Role::TextInput, "Grant access to");
    field.click();
    field.type_text("-fork");
    h.run();
    h.state_mut().handle_result(AppResult::Action(Err("clone failed".into())));
    h.run();
    assert_eq!(h.get_by_role_and_label(Role::TextInput, "Grant access to").value().unwrap_or_default(), "portal-fork");
}

#[test]
fn the_request_box_goes_away_when_the_request_is_resolved() {
    let mut a = app();
    a.handle_stream(snapshot(vec![waiting_for_portal("a1")]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("needs a repo").click();
    h.run();
    let mut resolved = waiting_for_portal("a1");
    resolved.repo_request = None;
    resolved.blocked_reason = None;
    resolved.status = Status::Working;
    h.state_mut().handle_stream(StreamMsg::Snapshot(Snapshot { seq: 2, agents: vec![resolved] }));
    h.run();
    assert!(h.query_by_label("Wants portal").is_none());
}

fn open_request(a: FleetApp) -> Harness<'static, FleetApp> {
    let mut a = a;
    a.handle_stream(snapshot(vec![waiting_for_portal("a1")]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("needs a repo").click();
    h.run();
    h
}

#[test]
fn a_second_grant_while_one_is_in_flight_is_not_sent() {
    let (a, hold) = hanging_app();
    hold.set_nonblocking(true).unwrap();
    let mut h = open_request(a);
    // Opening the preview connects the pane stream; only calls made after this point are counted.
    std::thread::sleep(Duration::from_millis(300));
    while hold.accept().is_ok() {}
    h.get_by_role_and_label(Role::Button, "Grant").click();
    h.run();
    assert!(h.state().deciding("a1"));
    h.get_by_role_and_label(Role::Button, "Grant").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Refuse").click();
    h.run();
    h.get_by_role_and_label(Role::TextInput, "Grant access to").click();
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run();
    std::thread::sleep(Duration::from_millis(300));
    let connections = std::iter::from_fn(|| hold.accept().ok()).count();
    assert_eq!(connections, 1, "only the first decision may reach the daemon");
    h.state_mut().handle_result(AppResult::RequestDecided("a1".into(), Ok(())));
    assert!(!h.state().deciding("a1"));
}

#[test]
fn a_failed_add_in_a_batch_stays_on_the_problem_line() {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent("a1", "work", Status::Working, &["core"])]));
    a.handle_result(AppResult::RepoAdded("a1".into(), Err("no such repo: nope".into())));
    a.handle_result(AppResult::RepoAdded("a1".into(), Ok(())));
    assert_eq!(a.problem(), Some("no such repo: nope"));
}

#[test]
fn a_batch_of_adds_is_sent_one_call_at_a_time() {
    let (a, hold) = hanging_app();
    hold.set_nonblocking(true).unwrap();
    let mut a = a;
    a.handle_stream(snapshot(vec![agent("a1", "work", Status::Working, &["core"])]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("work").click();
    h.run();
    std::thread::sleep(Duration::from_millis(300));
    while hold.accept().is_ok() {}
    h.get_by_role_and_label(Role::Button, "Add repo").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "core").click();
    h.get_by_role_and_label(Role::Button, "portal").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Add repos").click();
    h.run();
    h.run();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(std::iter::from_fn(|| hold.accept().ok()).count(), 1, "the second add waits for the first result");
}

/// The app stays open while the agent asks: only events reach it, never a fresh snapshot.
#[test]
fn a_live_request_event_shows_the_box_and_a_later_one_removes_it() {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent("a1", "needs a repo", Status::Working, &["core"])]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("needs a repo").click();
    h.run();
    assert!(h.query_by_label("Wants portal").is_none());
    let event = |seq, request: Option<RepoRequest>| {
        StreamMsg::Event(Event {
            seq,
            kind: EventKind::StatusChanged {
                id: AgentId("a1".into()),
                status: if request.is_some() { Status::NeedsYou } else { Status::Working },
                activity: "wants repo portal".into(),
                since: 1200,
                has_changes: false,
                paused: false,
                blocked_reason: request.as_ref().map(|_| BlockedReason::RepoAccess),
                repo_request: request,
            },
        })
    };
    h.state_mut().handle_stream(event(2, Some(RepoRequest { repo: "portal".into(), why: "for the login form".into() })));
    h.run();
    h.get_by_label("Wants portal");
    h.state_mut().handle_stream(event(3, None));
    h.run();
    assert!(h.query_by_label("Wants portal").is_none());
}

/// A daemon that answers `GET /api/repos` with `names` and counts how often it was asked.
fn repos_daemon(names: &'static [&'static str]) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use std::io::{BufRead, BufReader, Write};
    use std::sync::atomic::Ordering;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let asked = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = asked.clone();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut first = String::new();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            if reader.read_line(&mut first).is_err() {
                continue;
            }
            let body = if first.starts_with("GET /api/repos ") {
                count.fetch_add(1, Ordering::SeqCst);
                let repos: Vec<_> = names.iter().map(|n| repo_info(n)).collect();
                serde_json::to_string(&fleet_api::RepoList { repos }).unwrap()
            } else {
                "{}".to_string()
            };
            // Only the request line is read: every reply closes the connection.
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        }
    });
    (base, asked)
}

/// Repos are remembered by the daemon as they are attached, so the list fetched at startup goes stale.
#[test]
fn the_remembered_repos_are_fetched_again_after_anything_that_may_remember_one() {
    use std::sync::atomic::Ordering;
    let (base, asked) = repos_daemon(&["core", "fresh"]);
    let mut a = FleetApp::detached(FleetClient::new(&base), egui::Context::default());
    a.handle_result(AppResult::Schema(Ok(default_ui_schema())));
    let mut h = modal_harness(a);
    h.run();
    let results = [
        ("RepoAdded", AppResult::RepoAdded("a1".into(), Ok(()))),
        ("RequestDecided", AppResult::RequestDecided("a1".into(), Ok(()))),
        ("Spawn", AppResult::Spawn(Ok(agent("n", "new", Status::Queued, &[])))),
    ];
    for (name, result) in results {
        let before = asked.load(Ordering::SeqCst);
        h.state_mut().handle_result(result);
        let end = Instant::now() + Duration::from_secs(10);
        while asked.load(Ordering::SeqCst) == before && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(asked.load(Ordering::SeqCst) > before, "{name} did not refetch the repos");
    }
    // Opening the dialog fetches too, and its picks show what came back.
    let before = asked.load(Ordering::SeqCst);
    h.get_by_role_and_label(Role::Button, "New project").click();
    h.run();
    assert!(wait_for(&mut h, |_| asked.load(Ordering::SeqCst) > before), "opening the dialog did not refetch the repos");
    let end = Instant::now() + Duration::from_secs(10);
    while h.query_by_role_and_label(Role::Button, "fresh").is_none() && Instant::now() < end {
        h.run();
        std::thread::sleep(Duration::from_millis(50));
    }
    h.get_by_role_and_label(Role::Button, "fresh");
}

/// A project with no repo has no `repo / branch` to show; its folder is what there is. A grant then changes it.
#[test]
fn the_panel_path_is_the_project_folder_until_a_repo_arrives() {
    let mut a = app();
    let mut empty = agent("a1", "no repo yet", Status::Working, &[]);
    empty.workdir = "/projects/no-repo-yet".into();
    a.handle_stream(snapshot(vec![empty]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("no repo yet").click();
    h.run();
    h.get_by_label("/projects/no-repo-yet");
    h.state_mut().handle_stream(StreamMsg::Event(Event { seq: 2, kind: EventKind::WorkspacesChanged { id: AgentId("a1".into()), workspaces: vec![ws("core")] } }));
    h.run();
    h.get_by_label("core / fleet/x");
}
