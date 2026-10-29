mod common;

use eframe::egui::{self, accesskit::Role, Key, Modifiers, Vec2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use fleet_api::{
    default_ui_schema, Agent, AgentId, BlockedReason, DiffReport, Event, EventKind, RepoRequest,
    Snapshot, SpawnResponse, Status, StopCause, Workspace,
};
use fleet_client::http::{FleetClient, FleetError};
use fleet_client::stream::{PaneMsg, StreamMsg};
use fleet_gui::app::{AppResult, FleetApp, PersonaCall};
use fleet_gui::theme::{self, layout};
use fleet_gui::views::{settings::SettingsTab, View};
use fleet_log::Notice;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// `gui.toml` once every background write asked for so far has landed.
fn on_disk(path: &std::path::Path) -> fleet_client::config::GuiSettings {
    fleet_client::config::SettingsWriter::shared().flush();
    fleet_client::config::GuiSettings::load(path)
}

const NOW: i64 = 1300;

fn ws(repo: &str) -> Workspace {
    Workspace {
        repo: repo.into(),
        branch: "fleet/x".into(),
        worktree: format!("/w/{repo}"),
        base: "abc".into(),
        from_branch: None,
        has_changes: false,
        owned: false,
        changed: false,
    }
}

fn agent(id: &str, goal: &str, status: Status, repos: &[&str]) -> Agent {
    Agent {
        id: AgentId(id.into()),
        goal: goal.into(),
        workdir: "/w".into(),
        workspaces: repos.iter().map(|r| ws(r)).collect(),
        session: format!("fleet-{id}"),
        status,
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
        model: None,
        attach: None,
        mode: None,
        effort: None,
    }
}

/// An app that makes no daemon calls: tests feed it the results the background threads would send.
fn app() -> FleetApp {
    base_app(false)
}

/// `calls`: the app really calls its client (a closed port, so every call fails); for tests that check a call goes
/// out. Everywhere else a real answer would race the results the test injects.
fn base_app(calls: bool) -> FleetApp {
    let mut app = FleetApp::detached(
        FleetClient::new("http://127.0.0.1:1"),
        egui::Context::default(),
    );
    if !calls {
        app = app.without_calls();
    }
    app.set_clock(Box::new(|| NOW));
    app.handle_result(AppResult::Schema(Ok(default_ui_schema())));
    app.handle_result(AppResult::Repos(Ok(vec![
        repo_info("core"),
        repo_info("portal"),
    ])));
    app
}

/// An app whose log reaches it, for tests that look at what the footer was told. Keep the guard alive.
fn logged_app() -> (tracing::subscriber::DefaultGuard, FleetApp) {
    let (guard, handle) = common::log::test_log();
    (guard, app().with_log(handle))
}

/// `logged_app` whose calls really go out (and fail against the closed port).
fn logged_calling_app() -> (tracing::subscriber::DefaultGuard, FleetApp) {
    let (guard, handle) = common::log::test_log();
    (guard, base_app(true).with_log(handle))
}

fn notices(app: &FleetApp) -> Vec<(Notice, String)> {
    app.log_entries()
        .iter()
        .filter_map(|e| Some((e.notice()?, e.message.clone())))
        .collect()
}

fn has_notice(app: &FleetApp, notice: Notice, text: &str) -> bool {
    notices(app).contains(&(notice, text.to_string()))
}

fn has_error(app: &FleetApp) -> bool {
    notices(app).iter().any(|(n, _)| *n == Notice::Error)
}

fn snapshot(agents: Vec<Agent>) -> StreamMsg {
    StreamMsg::Snapshot(Snapshot { seq: 1, agents })
}

fn harness(app: FleetApp) -> Harness<'static, FleetApp> {
    Harness::builder()
        .with_size(Vec2::new(1400.0, 800.0))
        .build_ui_state(|ui, app: &mut FleetApp| app.ui(ui), app)
}

/// For tests that open the New agent dialog: a modal needs more frames than the default to settle.
fn modal_harness(app: FleetApp) -> Harness<'static, FleetApp> {
    Harness::builder()
        .with_max_steps(60)
        .with_size(Vec2::new(1400.0, 800.0))
        .build_ui_state(|ui, app: &mut FleetApp| app.ui(ui), app)
}

fn x(h: &Harness<'_, FleetApp>, label: &str) -> f32 {
    h.get_by_label(label).rect().center().x
}

fn status_changed(seq: u64, id: &str, status: Status) -> StreamMsg {
    StreamMsg::Event(Box::new(Event {
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
            stop_cause: None,
            disk_bytes: None,
        },
    }))
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
    a.handle_stream(snapshot(vec![agent(
        "a1",
        "fix the tests",
        Status::Working,
        &["core"],
    )]));
    let mut h = harness(a);
    h.run();
    let working = x(&h, "Working");
    assert!((x(&h, "fix the tests") - working).abs() < 200.0);
    h.state_mut()
        .handle_stream(status_changed(2, "a1", Status::NeedsYou));
    h.run();
    let needs = x(&h, "Needs you");
    assert!(
        (x(&h, "fix the tests") - needs).abs() < 200.0,
        "the card did not follow its status"
    );
    h.get_by_label("Permission needed");
}

#[test]
fn a_snapshot_replaces_the_board() {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent(
        "a1",
        "first",
        Status::Working,
        &["core"],
    )]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("first");
    h.state_mut().handle_stream(StreamMsg::Snapshot(Snapshot {
        seq: 9,
        agents: vec![agent("b", "second", Status::Review, &["core"])],
    }));
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
    h.get_by_role_and_label(Role::ComboBox, "Repo filter")
        .click();
    h.run();
    h.get_by_role_and_label(Role::CheckBox, "portal").click();
    h.run();
    assert!(
        h.query_by_label("core task").is_none(),
        "an agent outside the selected repo is shown"
    );
    h.get_by_label("portal task");
    h.get_by_label("both task");
    h.get_by_role_and_label(Role::CheckBox, "portal").click();
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
fn failed_action_reaches_the_footer() {
    let (_log, mut a) = logged_app();
    a.handle_stream(snapshot(vec![agent(
        "a1",
        "work",
        Status::Working,
        &["core"],
    )]));
    let mut h = harness(a);
    h.run();
    h.state_mut()
        .handle_result(AppResult::Action(Err("agent is not running".into())));
    h.run();
    h.get_by_label("agent is not running");
}

#[test]
fn running_an_action_goes_through_the_client_on_a_background_thread() {
    // The client points at a closed port, so the call fails; the failure must come back as a message.
    let (_log, mut a) = logged_calling_app();
    a.handle_stream(snapshot(vec![agent(
        "a1",
        "work",
        Status::Working,
        &["core"],
    )]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("Pause").click();
    h.run();
    assert!(
        wait_for(&mut h, has_error),
        "no result came back from the background call"
    );
}

#[test]
fn clicking_a_card_opens_a_read_only_preview_with_a_diff_tab() {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent(
        "a1",
        "open me",
        Status::Working,
        &["core"],
    )]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("open me").click();
    h.run();
    assert_eq!(h.state().preview_agent(), Some("a1"));
    h.get_by_label("Terminal");
    h.get_by_label("Diff");
    assert_eq!(
        h.query_all_by_role(Role::TextInput).count()
            + h.query_all_by_role(Role::MultilineTextInput).count(),
        0,
        "the preview must not add a typeable field"
    );
    h.state_mut().handle_pane(PaneMsg::Text("$ ready".into()));
    h.run();
    h.get_by_label_contains("$ ready");
}

#[test]
fn opening_a_needs_you_card_focuses_the_reply_box() {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent(
        "a1",
        "ask me",
        Status::NeedsYou,
        &["core"],
    )]));
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
    a.handle_stream(snapshot(vec![
        agent("a", "first", Status::NeedsYou, &["core"]),
        agent("b", "second", Status::NeedsYou, &["core"]),
    ]));
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
    assert_eq!(
        h.get_by_role(Role::MultilineTextInput)
            .value()
            .unwrap_or_default(),
        "draft a"
    );
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run();
    h.state_mut().handle_result(AppResult::Replied {
        agent_id: "a".into(),
        sent: "draft a".into(),
        result: Ok(None),
    });
    assert_eq!(h.state().reply_text("a"), "");
    assert_eq!(h.state().reply_text("b"), "draft b");
}

#[test]
fn a_failed_reply_stays_in_the_box() {
    let (_guard, mut a) = logged_app();
    a.handle_stream(snapshot(vec![agent(
        "a",
        "first",
        Status::NeedsYou,
        &["core"],
    )]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("first").click();
    h.run();
    h.get_by_role(Role::MultilineTextInput).type_text("yes, go");
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run();
    assert_eq!(
        h.state().reply_text("a"),
        "yes, go",
        "cleared before the daemon answered"
    );
    h.state_mut().handle_result(AppResult::Replied {
        agent_id: "a".into(),
        sent: "yes, go".into(),
        result: Err("agent is not running".into()),
    });
    h.run();
    assert_eq!(h.state().reply_text("a"), "yes, go");
    assert!(has_error(h.state()));
}

#[test]
fn a_reply_edited_while_sending_is_kept() {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent(
        "a",
        "first",
        Status::NeedsYou,
        &["core"],
    )]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("first").click();
    h.run();
    h.get_by_role(Role::MultilineTextInput).type_text("one");
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run();
    h.get_by_role(Role::MultilineTextInput).type_text(" two");
    h.run();
    h.state_mut().handle_result(AppResult::Replied {
        agent_id: "a".into(),
        sent: "one".into(),
        result: Ok(None),
    });
    assert_eq!(h.state().reply_text("a"), "one two");
}

#[test]
fn a_working_agent_has_no_reply_box() {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent(
        "a1",
        "busy",
        Status::Working,
        &["core"],
    )]));
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
    a.handle_stream(snapshot(vec![agent(
        "a1",
        "busy",
        Status::Working,
        &["core"],
    )]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("busy").click();
    h.run();
    h.state_mut()
        .handle_stream(status_changed(2, "a1", Status::NeedsYou));
    h.run();
    assert!(!h.get_by_role(Role::MultilineTextInput).is_focused());
}

#[test]
fn undo_in_one_agents_reply_box_cannot_bring_back_another_agents_draft() {
    let mut a = app();
    a.handle_stream(snapshot(vec![
        agent("a", "first", Status::NeedsYou, &["core"]),
        agent("b", "second", Status::NeedsYou, &["core"]),
    ]));
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
    assert_eq!(
        h.get_by_role(Role::MultilineTextInput)
            .value()
            .unwrap_or_default(),
        ""
    );
    assert_eq!(h.state().reply_text("b"), "");
    assert_eq!(h.state().reply_text("a"), "secret");
}

#[test]
fn opening_a_card_fills_the_panel_path() {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent(
        "a1",
        "open me",
        Status::Working,
        &["core"],
    )]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("open me").click();
    h.run();
    h.get_by_label("core / fleet/x");
}

#[test]
fn the_diff_tab_loads_through_the_app_and_shows_the_result() {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent(
        "a1",
        "open me",
        Status::Working,
        &["core"],
    )]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("open me").click();
    h.run();
    h.get_by_label("Diff").click();
    h.run();
    h.state_mut().handle_result(AppResult::Diff(
        "a1".into(),
        Ok(DiffReport {
            workspaces: vec![fleet_api::WorkspaceDiff {
                repo: "core".into(),
                diff: "+hello\n".into(),
                truncated: false,
            }],
        }),
    ));
    h.run();
    h.get_by_label_contains("+hello");
}

#[test]
fn the_preview_closes_when_its_agent_is_removed() {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent(
        "a1",
        "open me",
        Status::Working,
        &["core"],
    )]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("open me").click();
    h.run();
    assert!(h.state().preview_agent().is_some());
    h.state_mut()
        .handle_stream(StreamMsg::Event(Box::new(Event {
            seq: 2,
            kind: EventKind::AgentRemoved {
                id: AgentId("a1".into()),
            },
        })));
    h.run();
    assert_eq!(h.state().preview_agent(), None);
}

#[test]
fn closing_the_preview_drops_it() {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent(
        "a1",
        "open me",
        Status::Working,
        &["core"],
    )]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("open me").click();
    h.run();
    h.get_by_label("Close").click();
    h.run();
    assert_eq!(h.state().preview_agent(), None);
}

fn open_card_harness(width: f32) -> Harness<'static, FleetApp> {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent(
        "a1",
        "open me",
        Status::Working,
        &["core"],
    )]));
    let mut h = Harness::builder()
        .with_size(Vec2::new(width, 800.0))
        .build_ui_state(|ui, app: &mut FleetApp| app.ui(ui), a);
    h.run();
    h.get_by_label("open me").click();
    h.run();
    h
}

#[test]
fn fullscreen_button_hides_the_board() {
    let mut h = open_card_harness(1400.0);
    h.get_by_label("Needs you");
    h.get_by_label("Fullscreen").click();
    h.run();
    assert!(h.query_by_label("Needs you").is_none(), "the board is gone");
    h.get_by_label("Terminal");
    h.get_by_label("Back to the board").click();
    h.run();
    h.get_by_label("Needs you");
}

#[test]
fn esc_leaves_fullscreen_unless_typing() {
    let mut h = open_card_harness(1400.0);
    h.get_by_label("Fullscreen").click();
    h.run();
    h.key_press(Key::Escape);
    h.run();
    h.get_by_label("Needs you");

    let mut a = app();
    a.handle_stream(snapshot(vec![agent(
        "a1",
        "ask me",
        Status::NeedsYou,
        &["core"],
    )]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("ask me").click();
    h.run();
    h.get_by_label("Fullscreen").click();
    h.run();
    h.get_by_role(Role::MultilineTextInput).focus();
    h.run();
    assert!(h.get_by_role(Role::MultilineTextInput).is_focused());
    h.key_press(Key::Escape);
    h.run();
    assert!(
        h.query_by_label("Working").is_none(),
        "Esc while typing must not leave fullscreen"
    );
}

/// Esc over a dialog is the dialog's: it closes the dialog and leaves the chosen Full alone.
#[test]
fn esc_over_a_dialog_stays_fullscreen() {
    let (mut a, _hold) = hanging_app();
    a.handle_stream(snapshot(vec![agent(
        "a1",
        "open me",
        Status::Working,
        &["core"],
    )]));
    let mut h = modal_harness(a);
    h.run();
    h.get_by_label("open me").click();
    h.run();
    h.get_by_label("Fullscreen").click();
    h.run();
    assert!(h.query_by_label("Needs you").is_none(), "fullscreen");
    h.get_by_role_and_label(Role::Button, "New task").click();
    h.run();
    assert!(h.state().spawn_state().open);
    // No field has the keys: only the dialog may stop this Esc.
    h.ctx.memory_mut(|m| m.stop_text_input());
    h.run();
    h.key_press(Key::Escape);
    h.run();
    assert!(!h.state().spawn_state().open, "Esc closes the dialog");
    assert!(
        h.query_by_label("Needs you").is_none(),
        "Esc over a dialog must not leave fullscreen"
    );
}

#[test]
fn narrow_window_forces_full_width() {
    let mut h = open_card_harness(600.0);
    assert!(h.query_by_label("Needs you").is_none(), "the board is gone");
    h.get_by_label("Terminal");
    assert!(h
        .get_by_role_and_label(Role::Button, "Back to the board")
        .accesskit_node()
        .is_disabled());
    h.ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
    h.get_by_role_and_label(Role::Button, "Back to the board")
        .hover();
    h.run();
    h.get_by_label("the window is too narrow for the board");
    h.get_by_label("Close").click();
    h.run();
    h.get_by_label("Needs you");
}

#[test]
fn dragging_the_panel_wide_never_forces_full_width() {
    let mut h = open_card_harness(1400.0);
    let before = x(&h, "Terminal");
    let edge = egui::pos2(1400.0 - 420.0, 400.0);
    h.hover_at(edge);
    h.run();
    h.drag_at(edge);
    h.run();
    h.hover_at(egui::pos2(20.0, 400.0));
    h.run();
    h.drop_at(egui::pos2(20.0, 400.0));
    h.run();
    assert!(
        x(&h, "Terminal") < before - 100.0,
        "the drag widened the panel"
    );
    h.get_by_label("Needs you");
    assert!(!h
        .get_by_role_and_label(Role::Button, "Fullscreen")
        .accesskit_node()
        .is_disabled());
    // Closing and reopening hits the same width.
    h.get_by_label("Close").click();
    h.run();
    h.get_by_label("open me").click();
    h.run();
    h.get_by_label("Needs you");
    h.get_by_role_and_label(Role::Button, "Fullscreen");
}

/// An app whose spawn call hangs: a listener that accepts connections and never answers keeps it in flight for
/// the whole test, unlike a closed port, which would fail (and clear "busy") before the assertions look.
fn hanging_app() -> (FleetApp, std::net::TcpListener) {
    let hold = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut a = FleetApp::detached(
        FleetClient::new(&format!("http://{}", hold.local_addr().unwrap())),
        egui::Context::default(),
    )
    .with_product("p");
    a.handle_result(AppResult::Schema(Ok(default_ui_schema())));
    a.handle_result(AppResult::Repos(Ok(vec![
        repo_info("core"),
        repo_info("portal"),
    ])));
    a.handle_result(personas(vec![persona("p", "Personal")]));
    (a, hold)
}

/// The repos Personal remembers, as the dialog's fetch would bring them.
fn personal_repos(h: &mut Harness<'static, FleetApp>) {
    h.state_mut().handle_result(AppResult::PersonaRepos(
        "p".into(),
        Ok(vec![repo_info("core"), repo_info("portal")]),
    ));
    h.run();
}

/// Opens the dialog, types a prompt and starts the agent, leaving the spawn in flight.
fn start_an_agent(h: &mut Harness<'static, FleetApp>) {
    h.run();
    h.get_by_role_and_label(Role::Button, "New task").click();
    h.run();
    let prompt = h.get_by_role(Role::MultilineTextInput);
    prompt.click();
    prompt.type_text("do the thing");
    h.run();
    h.get_by_role_and_label(Role::Button, "Start task").click();
    h.run();
    assert!(h.state().spawn_state().busy, "a spawn is in flight");
}

#[test]
fn spawn_success_clears_the_prompt_and_failure_keeps_it_with_the_error() {
    let (a, _hold) = hanging_app();
    let mut h = modal_harness(a);
    start_an_agent(&mut h);
    h.state_mut()
        .handle_result(AppResult::Spawn(Err(FleetError::new("unknown repo \"x\""))));
    h.run();
    h.get_by_label("unknown repo \"x\"");
    assert_eq!(h.state().spawn_state().prompt, "do the thing");
    assert!(!h.state().spawn_state().busy);
    assert!(
        h.state().spawn_state().open,
        "a failure keeps the dialog open"
    );
    assert!(
        !has_error(h.state()),
        "the dialog is showing the error already"
    );
    h.state_mut()
        .handle_result(AppResult::Spawn(Ok(SpawnResponse {
            agent: agent("n", "new", Status::Queued, &["core"]),
            warnings: vec![],
        })));
    h.run();
    assert_eq!(h.state().spawn_state().prompt, "");
    assert_eq!(h.state().spawn_state().error, None);
    assert!(!h.state().spawn_state().open);
}

#[test]
fn a_spawn_refused_for_a_tool_shows_its_fix_and_the_wizard_link() {
    let (a, _hold) = hanging_app();
    let checks = std::sync::Arc::new(std::sync::Mutex::new(0));
    let counted = checks.clone();
    let seams = fleet_gui::tool_check::ToolSeams {
        recheck: Some(std::sync::Arc::new(move || {
            *counted.lock().unwrap() += 1;
            Ok(fleet_api::ToolList {
                tools: vec![],
                checked_at: 1,
                path: None,
            })
        })),
        install: Some(std::sync::Arc::new(|_: &str| Ok(()))),
        ..Default::default()
    };
    let mut h = modal_harness(a.with_tool_seams(seams));
    start_an_agent(&mut h);
    let missing = fleet_api::ToolError::NotInstalled {
        tool: fleet_api::ToolId::Claude,
        fix: fleet_api::ToolFix {
            install: Some("irm https://claude.ai/install.ps1 | iex".into()),
            link: "https://claude.com/claude-code".into(),
            manager: None,
        },
    };
    h.state_mut()
        .handle_result(AppResult::Spawn(Err(FleetError {
            message: missing.to_string(),
            tool: Some(Box::new(missing)),
        })));
    h.run();
    assert!(h.state().spawn_state().open, "the dialog stays open");
    h.get_by_label("Claude Code is not installed");
    h.get_by_label("irm https://claude.ai/install.ps1 | iex");
    h.get_by_label("https://claude.com/claude-code");
    assert!(
        h.query_by_label("Install").is_none(),
        "installing happens only in the wizard"
    );
    h.get_by_role_and_label(Role::Button, "Open the setup wizard")
        .click();
    h.run();
    assert!(
        !h.state().spawn_state().open,
        "the wizard replaces the dialog"
    );
    h.get_by_label_contains("What Fleet Manager needs");
    let end = Instant::now() + Duration::from_secs(10);
    while *checks.lock().unwrap() == 0 && Instant::now() < end {
        h.step();
    }
    assert_eq!(*checks.lock().unwrap(), 1, "the wizard checks the tools");
}

/// A lapsed login is fixed from the persona's setup, not by installing: no way to the setup wizard.
#[test]
fn a_spawn_refused_for_a_login_does_not_offer_the_wizard() {
    let (a, _hold) = hanging_app();
    let mut h = modal_harness(a);
    start_an_agent(&mut h);
    let logged_out = fleet_api::ToolError::NotLoggedIn {
        tool: fleet_api::ToolId::Gh,
        persona: Some("Work".into()),
    };
    h.state_mut()
        .handle_result(AppResult::Spawn(Err(FleetError {
            message: logged_out.to_string(),
            tool: Some(Box::new(logged_out)),
        })));
    h.run();
    h.get_by_label("GitHub CLI (gh) is not logged in for Work");
    assert!(h
        .query_by_role_and_label(Role::Button, "Open the setup wizard")
        .is_none());
}

#[test]
fn spawn_failure_after_closing_the_dialog_shows_in_the_footer() {
    let (_log, handle) = common::log::test_log();
    let (a, _hold) = hanging_app();
    let a = a.with_log(handle);
    let mut h = modal_harness(a);
    start_an_agent(&mut h);
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(!h.state().spawn_state().open);
    h.state_mut()
        .handle_result(AppResult::Spawn(Err(FleetError::new("boom"))));
    h.run();
    assert!(has_notice(
        h.state(),
        Notice::Error,
        "Could not start the agent: boom"
    ));
    assert!(
        !h.state().spawn_state().open,
        "a failure must not reopen the dialog"
    );
    assert_eq!(h.state().spawn_state().error.as_deref(), Some("boom"));
    h.get_by_label("Could not start the agent: boom");
}

#[test]
fn a_successful_spawn_clears_the_chosen_repos() {
    let (a, _hold) = hanging_app();
    let mut h = modal_harness(a);
    h.run();
    h.get_by_role_and_label(Role::Button, "New task").click();
    h.run();
    personal_repos(&mut h);
    h.get_by_role_and_label(Role::Button, "core").click();
    // The path-only repo adds its start line, which moves the centred dialog.
    h.run();
    let goal = h.get_by_role(Role::MultilineTextInput);
    goal.click();
    goal.type_text("go");
    h.run();
    h.get_by_role_and_label(Role::Button, "Start task").click();
    h.run();
    h.state_mut()
        .handle_result(AppResult::Spawn(Ok(SpawnResponse {
            agent: agent("n", "new", Status::Queued, &["core"]),
            warnings: vec![],
        })));
    h.run();
    assert!(h.state().spawn_state().picker.chosen.is_empty());
}

#[test]
fn reopening_the_dialog_keeps_the_chosen_repos() {
    let (a, _hold) = hanging_app();
    let mut h = modal_harness(a);
    h.run();
    h.get_by_role_and_label(Role::Button, "New task").click();
    h.run();
    personal_repos(&mut h);
    h.get_by_role_and_label(Role::Button, "core").click();
    h.run();
    h.key_press(egui::Key::Escape);
    h.run();
    h.get_by_role_and_label(Role::Button, "New task").click();
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
    h.state_mut()
        .handle_result(AppResult::Spawn(Ok(SpawnResponse {
            agent: agent("n", "new", Status::Queued, &["core"]),
            warnings: vec![],
        })));
    h.run();
    assert!(!h.state().spawn_state().open);
    assert_eq!(h.state().spawn_state().prompt, "");
}

#[test]
fn a_started_agent_is_announced() {
    let (_log, handle) = common::log::test_log();
    let (a, _hold) = hanging_app();
    let mut h = modal_harness(a.with_log(handle));
    start_an_agent(&mut h);
    h.state_mut()
        .handle_result(AppResult::Spawn(Ok(SpawnResponse {
            agent: agent("n", "new", Status::Queued, &["core"]),
            warnings: vec![],
        })));
    h.run();
    assert!(has_notice(h.state(), Notice::Success, "Agent started"));
    h.get_by_label("Agent started");
}

#[test]
fn spawn_warnings_are_announced_after_the_success() {
    let (_log, handle) = common::log::test_log();
    let (a, _hold) = hanging_app();
    let mut h = modal_harness(a.with_log(handle));
    start_an_agent(&mut h);
    let warnings = vec!["could not fetch core: network unreachable".to_string()];
    h.state_mut()
        .handle_result(AppResult::Spawn(Ok(SpawnResponse {
            agent: agent("n", "new", Status::Queued, &["core"]),
            warnings,
        })));
    h.run();
    assert!(has_notice(h.state(), Notice::Success, "Agent started"));
    assert!(has_notice(
        h.state(),
        Notice::Warning,
        "could not fetch core: network unreachable"
    ));
    let order: Vec<_> = notices(h.state()).into_iter().map(|(n, _)| n).collect();
    assert_eq!(order, [Notice::Success, Notice::Warning]);
}

#[test]
fn spawn_success_keeps_a_dialog_reopened_and_typed_into_while_in_flight() {
    let (a, _hold) = hanging_app();
    let mut h = modal_harness(a);
    start_an_agent(&mut h);
    h.key_press(egui::Key::Escape);
    h.run();
    h.get_by_role_and_label(Role::Button, "New task").click();
    h.run();
    h.get_by_role(Role::MultilineTextInput)
        .type_text(" and another");
    h.run();
    let typed = h.state().spawn_state().prompt.clone();
    assert_ne!(typed, "do the thing", "the reopened prompt was typed into");
    h.state_mut()
        .handle_result(AppResult::Spawn(Ok(SpawnResponse {
            agent: agent("n", "new", Status::Queued, &["core"]),
            warnings: vec![],
        })));
    h.run();
    assert!(
        h.state().spawn_state().open,
        "the dialog the user is typing in must stay open"
    );
    assert_eq!(
        h.state().spawn_state().prompt,
        typed,
        "the new text must be kept"
    );
    assert!(!h.state().spawn_state().busy);
}

#[test]
fn schema_load_failure_shows_the_error_and_a_retry() {
    let mut a = FleetApp::detached(
        FleetClient::new("http://127.0.0.1:1"),
        egui::Context::default(),
    );
    a.handle_result(AppResult::Schema(Err("connection refused".into())));
    let mut h = harness(a);
    h.run();
    h.get_by_label_contains("connection refused");
    h.get_by_label("Retry").click();
    h.run();
    assert!(
        h.query_by_label_contains("connection refused").is_none(),
        "retry clears the old error while it loads"
    );
}

#[test]
fn the_ui_does_not_block_while_the_daemon_is_unreachable() {
    // The real constructor starts the background threads against a port nothing listens on.
    let mut h = harness(FleetApp::new(
        FleetClient::new("http://127.0.0.1:1"),
        egui::Context::default(),
        PathBuf::new(),
        PathBuf::new(),
        PathBuf::new(),
        PathBuf::new(),
    ));
    let t = Instant::now();
    for _ in 0..10 {
        h.step();
    }
    assert!(
        t.elapsed() < Duration::from_millis(500),
        "ten frames took {:?}",
        t.elapsed()
    );
    assert!(
        wait_for(&mut h, |app| app.load_error().is_some()),
        "the failed load was never reported"
    );
}

/// The colour a viewer sees at `p`: the last shape painted over it (filled rect or line), in paint order.
fn colour_at(h: &Harness<'_, FleetApp>, p: egui::Pos2) -> Option<egui::Color32> {
    fn walk(shape: &egui::Shape, p: egui::Pos2, top: &mut Option<egui::Color32>) {
        match shape {
            egui::Shape::Rect(r) if r.fill.a() > 0 && r.rect.contains(p) => *top = Some(r.fill),
            egui::Shape::LineSegment {
                points: [a, b],
                stroke,
            } => {
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
    h.output()
        .shapes
        .iter()
        .for_each(|c| walk(&c.shape, &mut all));
    all.into_iter()
        .filter(|(r, f)| *f == fill && r.contains(p))
        .map(|(r, _)| r)
        .min_by(|a, b| a.area().total_cmp(&b.area()))
        .expect("a frame of that fill around the point")
}

#[test]
fn column_headers_are_flush_and_bordered_and_dividers_run_full_height() {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent(
        "a1",
        "work",
        Status::Working,
        &["core"],
    )]));
    let mut h = harness(a);
    h.run();
    let t = theme::tokens(egui::Visuals::default().dark_mode);
    let centre = |label: &str| h.get_by_label(label).rect().center();
    // The content header spans the board; the board ends where the content footer starts.
    let top_bar = frame_around(
        &h,
        h.get_by_role_and_label(Role::Label, "Kanban")
            .rect()
            .center(),
        t.surface,
    );
    let bottom = h.state().layout_rects().footer.top();
    let header = frame_around(&h, centre("Stopped"), t.surface);
    let col_w = top_bar.width() / 5.0;

    assert!(
        header.left() - top_bar.left() <= 0.5 && header.width() >= col_w - 1.0,
        "header is not full column width: {header:?}"
    );
    // The strip under the top bar spans the board; the column headers sit flush under it.
    let strip = h.state().layout_rects().strip;
    assert!(
        (strip.top() - top_bar.bottom()).abs() <= 1.0,
        "the strip floats under the top bar: {strip:?} vs {top_bar:?}"
    );
    assert!(
        (header.top() - strip.bottom()).abs() <= 1.0,
        "header floats under the strip: {header:?} vs {strip:?}"
    );
    // The whole last pixel row: a line centred on the edge is half covered by the body and reads as no line.
    for dy in [0.9, 0.1] {
        assert_eq!(
            colour_at(&h, egui::pos2(header.center().x, header.bottom() - dy)),
            Some(t.border),
            "no full rule under the header"
        );
    }

    for i in 1..5 {
        let x = top_bar.left() + col_w * i as f32;
        for y in [strip.bottom() + 1.5, bottom - 1.0] {
            assert_eq!(
                colour_at(&h, egui::pos2(x, y)),
                Some(t.border),
                "divider {i} missing at y={y}"
            );
        }
    }
}

#[test]
fn egui_transitions_follow_the_gate() {
    let mut h = harness(app());
    h.step();
    assert_eq!(h.ctx.global_style().animation_time, 0.0);

    h.input_mut()
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .focused = Some(true);
    h.step();
    assert_eq!(h.ctx.global_style().animation_time, theme::ANIMATION_TIME);

    h.input_mut()
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .focused = Some(false);
    h.step();
    assert_eq!(h.ctx.global_style().animation_time, 0.0);
}

fn repo_info(name: &str) -> fleet_api::RepoInfo {
    fleet_api::RepoInfo {
        name: name.into(),
        product: "p".into(),
        path: Some(format!("/repos/{name}")),
        // A URL, so a task starts from it: the fixture's path is not a checkout to list.
        url: Some(format!("https://example.invalid/{name}.git")),
        workflow: fleet_api::WorkflowState::Missing {
            base: "main".into(),
        },
    }
}

fn waiting_for_portal(id: &str) -> Agent {
    let mut a = agent(id, "needs a repo", Status::NeedsYou, &["core"]);
    a.blocked_reason = Some(BlockedReason::RepoAccess);
    a.repo_request = Some(RepoRequest {
        repo: "portal".into(),
        why: "for the login form".into(),
    });
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
    assert!(h
        .get_by_role_and_label(Role::TextInput, "Grant access to")
        .is_focused());
    // On the card and in the panel's details.
    assert_eq!(h.query_all_by_label("Repo access").count(), 2);
}

#[test]
fn grant_and_refuse_go_through_the_client_and_failures_show() {
    for (button, who) in [("Grant", "grant"), ("Refuse", "refuse")] {
        let (_log, mut a) = logged_calling_app();
        a.handle_stream(snapshot(vec![waiting_for_portal("a1")]));
        let mut h = harness(a);
        h.run();
        h.get_by_label("needs a repo").click();
        h.run();
        h.get_by_role_and_label(Role::Button, button).click();
        assert!(
            wait_for(&mut h, has_error),
            "{who}: the closed port's failure never came back"
        );
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
    h.state_mut()
        .handle_result(AppResult::Action(Err("clone failed".into())));
    h.run();
    assert_eq!(
        h.get_by_role_and_label(Role::TextInput, "Grant access to")
            .value()
            .unwrap_or_default(),
        "portal-fork"
    );
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
    h.state_mut().handle_stream(StreamMsg::Snapshot(Snapshot {
        seq: 2,
        agents: vec![resolved],
    }));
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
    // Kept open: a dropped connection fails the snapshot's tasks fetch, and the retry would be counted.
    let _open: Vec<_> = std::iter::from_fn(|| hold.accept().ok()).collect();
    h.get_by_role_and_label(Role::Button, "Grant").click();
    h.run();
    assert!(h.state().deciding("a1"));
    h.get_by_role_and_label(Role::Button, "Grant").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Refuse").click();
    h.run();
    h.get_by_role_and_label(Role::TextInput, "Grant access to")
        .click();
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run();
    std::thread::sleep(Duration::from_millis(300));
    let connections = std::iter::from_fn(|| hold.accept().ok()).count();
    assert_eq!(
        connections, 1,
        "only the first decision may reach the daemon"
    );
    h.state_mut()
        .handle_result(AppResult::RequestDecided("a1".into(), Ok(())));
    assert!(!h.state().deciding("a1"));
}

/// Only a grant is announced: a refusal that succeeds says nothing in the footer.
#[test]
fn repo_granted_is_announced_for_a_grant_and_not_for_a_refusal() {
    for (button, announced) in [("Grant", true), ("Refuse", false)] {
        let (log, handle) = common::log::test_log();
        let (a, _hold) = hanging_app();
        let mut h = open_request(a.with_log(handle));
        h.get_by_role_and_label(Role::Button, button).click();
        h.run();
        assert!(
            h.state().deciding("a1"),
            "{button}: the decision is in flight"
        );
        h.state_mut()
            .handle_result(AppResult::RequestDecided("a1".into(), Ok(())));
        h.run();
        assert_eq!(
            has_notice(h.state(), Notice::Success, "Repo granted"),
            announced,
            "{button}"
        );
        drop(log);
    }
}

#[test]
fn a_failed_add_in_a_batch_is_not_wiped_by_the_next_success() {
    let (_log, a) = logged_app();
    let mut h = harness(a);
    h.state_mut().handle_stream(snapshot(vec![agent(
        "a1",
        "work",
        Status::Working,
        &["core"],
    )]));
    h.state_mut().handle_result(AppResult::RepoAdded(
        "a1".into(),
        Err("no such repo: nope".into()),
    ));
    h.state_mut()
        .handle_result(AppResult::RepoAdded("a1".into(), Ok(())));
    h.run();
    assert_eq!(
        notices(h.state()),
        vec![
            (Notice::Error, "no such repo: nope".to_string()),
            (Notice::Success, "Repo added".to_string())
        ]
    );
    h.get_by_label("no such repo: nope");
}

#[test]
fn a_background_result_reaches_the_footer_on_the_frame_that_handles_it() {
    let (_log, mut a) = logged_calling_app();
    a.handle_stream(snapshot(vec![waiting_for_portal("a1")]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("needs a repo").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Refuse").click();
    h.step();
    assert!(h.state().deciding("a1"), "the refusal is in flight");
    let end = Instant::now() + Duration::from_secs(10);
    while h.state().deciding("a1") && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
        h.step();
    }
    assert!(
        !h.state().deciding("a1"),
        "the closed port's failure never came back"
    );
    // No further frame: nothing wakes a quiet window to read the log later.
    let (_, text) = notices(h.state())
        .pop()
        .expect("the failure was not logged on the frame that handled it");
    h.get_by_label(&text);
}

#[test]
fn only_the_apps_own_events_reach_the_footer() {
    let (_log, a) = logged_app();
    let mut h = harness(a);
    tracing::error!(target: "hyper::client", notify = "error", "from a dependency");
    tracing::error!(target: "fleet_gui::app", notify = "error", "from the app");
    h.run();
    h.get_by_label("from the app");
    assert!(h.query_by_label("from a dependency").is_none());
}

#[test]
fn long_message_is_one_line() {
    let (_log, a) = logged_app();
    let mut h = harness(a);
    let long = format!("first line\n{}\nlast", "word ".repeat(80));
    h.state_mut().handle_result(AppResult::Action(Err(long)));
    h.run();
    let label = h.get_by_label_contains("first line");
    assert!(
        label.rect().height() <= layout::FOOTER,
        "the message took {} px",
        label.rect().height()
    );
    assert!(
        label.rect().width() < 1400.0 - 200.0,
        "it must stay inside the content column"
    );
}

#[test]
fn open_log_switches_view() {
    let (_log, a) = logged_app();
    let mut h = harness(a);
    theme::apply(&h.ctx);
    h.state_mut()
        .handle_result(AppResult::Action(Err("boom".into())));
    h.run();
    h.get_by_role_and_label(Role::Button, "History").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Open log →").click();
    h.run();
    assert_eq!(h.state().view(), View::Settings);
    assert_eq!(h.state().settings_tab(), SettingsTab::Log);
}

#[test]
fn a_batch_of_adds_is_sent_one_call_at_a_time() {
    let (a, hold) = hanging_app();
    hold.set_nonblocking(true).unwrap();
    let mut a = a;
    a.handle_stream(snapshot(vec![agent(
        "a1",
        "work",
        Status::Working,
        &["core"],
    )]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("work").click();
    h.run();
    std::thread::sleep(Duration::from_millis(300));
    // Kept open: a dropped connection fails the snapshot's tasks fetch, and the retry would be counted.
    let _open: Vec<_> = std::iter::from_fn(|| hold.accept().ok()).collect();
    h.get_by_role_and_label(Role::Button, "Add repo").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "core").click();
    h.get_by_role_and_label(Role::Button, "portal").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Add repos").click();
    h.run();
    h.run();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        std::iter::from_fn(|| hold.accept().ok()).count(),
        1,
        "the second add waits for the first result"
    );
}

/// The app stays open while the agent asks: only events reach it, never a fresh snapshot.
#[test]
fn a_live_request_event_shows_the_box_and_a_later_one_removes_it() {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent(
        "a1",
        "needs a repo",
        Status::Working,
        &["core"],
    )]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("needs a repo").click();
    h.run();
    assert!(h.query_by_label("Wants portal").is_none());
    let event = |seq, request: Option<RepoRequest>| {
        StreamMsg::Event(Box::new(Event {
            seq,
            kind: EventKind::StatusChanged {
                id: AgentId("a1".into()),
                status: if request.is_some() {
                    Status::NeedsYou
                } else {
                    Status::Working
                },
                activity: "wants repo portal".into(),
                since: 1200,
                has_changes: false,
                paused: false,
                blocked_reason: request.as_ref().map(|_| BlockedReason::RepoAccess),
                repo_request: request,
                stop_cause: None,
                disk_bytes: None,
            },
        }))
    };
    h.state_mut().handle_stream(event(
        2,
        Some(RepoRequest {
            repo: "portal".into(),
            why: "for the login form".into(),
        }),
    ));
    h.run();
    h.get_by_label("Wants portal");
    h.state_mut().handle_stream(event(3, None));
    h.run();
    assert!(h.query_by_label("Wants portal").is_none());
}

/// A daemon that answers `GET /api/repos` with `names` and counts how often it was asked.
fn repos_daemon(
    names: &'static [&'static str],
) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
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
            // `GET /api/repos?product=…` too: the dialog's own list, of the active product.
            let body = if first.starts_with("GET /api/products") {
                r#"{"products": [{"id": "p", "name": "p", "default_persona": null, "repos": [], "releases": [], "tasks": 0}]}"#.to_string()
            } else if first.starts_with("GET /api/repos") {
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
    let mut a =
        FleetApp::detached(FleetClient::new(&base), egui::Context::default()).with_product("p");
    a.handle_result(AppResult::Schema(Ok(default_ui_schema())));
    a.handle_result(personas(vec![persona("p", "Personal")]));
    let mut h = modal_harness(a);
    h.run();
    let results = [
        ("RepoAdded", AppResult::RepoAdded("a1".into(), Ok(()))),
        (
            "RequestDecided",
            AppResult::RequestDecided("a1".into(), Ok(())),
        ),
        (
            "Spawn",
            AppResult::Spawn(Ok(SpawnResponse {
                agent: agent("n", "new", Status::Queued, &[]),
                warnings: vec![],
            })),
        ),
    ];
    for (name, result) in results {
        let before = asked.load(Ordering::SeqCst);
        h.state_mut().handle_result(result);
        let end = Instant::now() + Duration::from_secs(10);
        while asked.load(Ordering::SeqCst) == before && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            asked.load(Ordering::SeqCst) > before,
            "{name} did not refetch the repos"
        );
    }
    // Opening the dialog fetches too, and its picks show what came back.
    let before = asked.load(Ordering::SeqCst);
    h.get_by_role_and_label(Role::Button, "New task").click();
    h.run();
    assert!(
        wait_for(&mut h, |_| asked.load(Ordering::SeqCst) > before),
        "opening the dialog did not refetch the repos"
    );
    let end = Instant::now() + Duration::from_secs(10);
    while h.query_by_role_and_label(Role::Button, "fresh").is_none() && Instant::now() < end {
        h.run();
        std::thread::sleep(Duration::from_millis(50));
    }
    h.get_by_role_and_label(Role::Button, "fresh");
}

/// A task with no repo has no `repo / branch` to show; its folder is what there is. A grant then changes it.
#[test]
fn the_panel_path_is_the_task_folder_until_a_repo_arrives() {
    let mut a = app();
    let mut empty = agent("a1", "no repo yet", Status::Working, &[]);
    empty.workdir = "/tasks/no-repo-yet".into();
    a.handle_stream(snapshot(vec![empty]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("no repo yet").click();
    h.run();
    h.get_by_label("/tasks/no-repo-yet");
    h.state_mut()
        .handle_stream(StreamMsg::Event(Box::new(Event {
            seq: 2,
            kind: EventKind::WorkspacesChanged {
                id: AgentId("a1".into()),
                workspaces: vec![ws("core")],
            },
        })));
    h.run();
    h.get_by_label("core / fleet/x");
}

#[test]
fn app_entries_are_collected() {
    let (_guard, handle) = common::log::test_log();
    let mut h = harness(app().with_log(handle));
    tracing::info!(target: "fleet_gui::app", "hello");
    h.step();
    assert_eq!(h.state().log_entries().last().unwrap().message, "hello");
}

#[test]
fn file_error_is_noticed() {
    let (_guard, handle) = common::log::test_log_without_file();
    assert!(handle.file_error().is_some());
    let mut h = harness(app().with_log(handle));
    h.step();
    h.step();
    let notices: Vec<_> = h
        .state()
        .log_entries()
        .iter()
        .filter(|e| e.message.starts_with("Logging to file is off"))
        .collect();
    assert_eq!(notices.len(), 1, "once, not every frame");
    assert_eq!(notices[0].notice(), Some(fleet_log::Notice::Warning));
}

#[test]
fn log_is_capped_dropping_the_oldest() {
    let (_guard, handle) = common::log::test_log();
    let mut h = harness(app().with_log(handle));
    for i in 0..5_010 {
        tracing::info!(target: "fleet_gui::app", "n{i}");
    }
    h.step();
    let log = h.state().log_entries();
    assert_eq!(log.len(), 5_000);
    assert_eq!(log[0].message, "n10");
    assert_eq!(log.last().unwrap().message, "n5009");
}

#[test]
fn a_detached_app_has_no_entries() {
    assert!(app().log_entries().is_empty());
}

#[test]
fn the_footer_checkbox_is_on_by_default_and_saved_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gui.toml");
    let mut h = harness(app().with_settings(path.clone()));
    h.run();
    h.get_by_role_and_label(Role::Button, "Settings").click();
    h.run();
    h.get_by_label("Footer");
    let keep = "Keep errors and warnings in the footer until dismissed";
    assert_eq!(
        h.get_by_label(keep).accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True),
        "on by default"
    );
    h.get_by_label(keep).click();
    h.run();
    assert!(!on_disk(&path).sticky_alerts);
}

fn stopped_event(seq: u64, id: &str, cause: StopCause, activity: &str) -> StreamMsg {
    StreamMsg::Event(Box::new(Event {
        seq,
        kind: EventKind::StatusChanged {
            id: AgentId(id.into()),
            status: Status::Stopped,
            activity: activity.into(),
            since: 1200,
            has_changes: false,
            paused: false,
            blocked_reason: None,
            repo_request: None,
            stop_cause: Some(cause),
            disk_bytes: None,
        },
    }))
}

#[test]
fn a_crash_posts_a_warning_naming_the_task_and_last_activity() {
    let (_log, mut a) = logged_app();
    a.handle_stream(snapshot(vec![agent(
        "a1",
        "fix login",
        Status::Working,
        &["core"],
    )]));
    let mut h = harness(a);
    h.run();
    h.state_mut().handle_stream(stopped_event(
        2,
        "a1",
        StopCause::Crashed,
        "crashed while running Bash",
    ));
    h.run();
    assert_eq!(
        notices(h.state()),
        vec![(
            Notice::Warning,
            "fix login crashed: running Bash".to_string()
        )]
    );
    h.get_by_label("fix login crashed: running Bash");
    // Task 10 makes a click on it select the card, so the message must say which agent it is about.
    let entry = h
        .state()
        .log_entries()
        .iter()
        .find(|e| e.notice().is_some())
        .unwrap()
        .clone();
    assert!(
        entry
            .fields
            .iter()
            .any(|(k, v)| k == "agent_id" && v == "a1"),
        "the warning does not carry the agent id: {:?}",
        entry.fields
    );
}

#[test]
fn an_agent_that_exits_posts_an_info_line() {
    let (_log, mut a) = logged_app();
    a.handle_stream(snapshot(vec![agent(
        "a1",
        "fix login",
        Status::Working,
        &["core"],
    )]));
    a.handle_stream(stopped_event(2, "a1", StopCause::Exited, "agent exited"));
    let mut h = harness(a);
    h.run();
    assert_eq!(
        notices(h.state()),
        vec![(Notice::Info, "fix login exited".to_string())]
    );
}

#[test]
fn a_stop_by_the_user_says_nothing_from_the_stream() {
    // The Stop action's own result announces it; the event must not say it twice.
    let (_log, mut a) = logged_app();
    a.handle_stream(snapshot(vec![agent(
        "a1",
        "fix login",
        Status::Working,
        &["core"],
    )]));
    a.handle_stream(stopped_event(2, "a1", StopCause::Stopped, "stopped by you"));
    let mut h = harness(a);
    h.run();
    assert!(notices(h.state()).is_empty(), "{:?}", notices(h.state()));
}

#[test]
fn an_agent_a_restart_could_not_resume_raises_no_notice() {
    let (_log, mut a) = logged_app();
    a.handle_stream(snapshot(vec![agent(
        "a1",
        "fix login",
        Status::Working,
        &["core"],
    )]));
    a.handle_stream(stopped_event(
        2,
        "a1",
        StopCause::Interrupted,
        "could not resume: claude exited at once",
    ));
    let mut h = harness(a);
    h.run();
    assert!(notices(h.state()).is_empty(), "{:?}", notices(h.state()));
}

#[test]
fn agents_already_stopped_in_a_snapshot_post_nothing() {
    let (_log, mut a) = logged_app();
    let mut gone = agent("a1", "fix login", Status::Stopped, &["core"]);
    gone.stop_cause = Some(StopCause::Crashed);
    a.handle_stream(snapshot(vec![gone]));
    // A repeated event for an agent that was already stopped is not a new crash either.
    a.handle_stream(stopped_event(
        2,
        "a1",
        StopCause::Crashed,
        "crashed while running Bash",
    ));
    let mut h = harness(a);
    h.run();
    assert!(notices(h.state()).is_empty(), "{:?}", notices(h.state()));
}

#[test]
fn a_finished_stop_posts_a_success_line() {
    let (_log, a) = logged_app();
    let mut h = harness(a);
    h.state_mut().handle_result(AppResult::Stopped {
        goal: "fix login".into(),
        result: Ok(None),
    });
    h.run();
    assert_eq!(
        notices(h.state()),
        vec![(Notice::Success, "fix login stopped".to_string())]
    );
    h.get_by_label("fix login stopped");
    h.state_mut().handle_result(AppResult::Stopped {
        goal: "fix login".into(),
        result: Err("agent is gone".into()),
    });
    h.run();
    assert!(has_notice(h.state(), Notice::Error, "agent is gone"));
}

#[test]
fn a_stop_click_reports_through_the_stop_result() {
    // The closed port fails the call; what comes back must be the Stop's error, not a success.
    let (_log, mut a) = logged_calling_app();
    a.handle_stream(snapshot(vec![agent(
        "a1",
        "fix login",
        Status::Working,
        &["core"],
    )]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("Stop").click();
    h.run();
    h.get_by_label("Click again to stop").click();
    h.run();
    assert!(wait_for(&mut h, has_error), "the Stop never came back");
    assert!(!has_notice(h.state(), Notice::Success, "fix login stopped"));
}

#[test]
fn a_stopped_agents_panel_has_no_reply_box() {
    let mut a = app();
    // A request left over from before the stop: nobody is waiting on it any more.
    let mut gone = waiting_for_portal("a1");
    gone.status = Status::Stopped;
    gone.stop_cause = Some(StopCause::Stopped);
    a.handle_stream(snapshot(vec![gone]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("needs a repo").click();
    h.run();
    assert_eq!(h.state().preview_agent(), Some("a1"));
    assert!(h.query_by_role(Role::MultilineTextInput).is_none());
    assert!(
        h.query_by_label("Wants portal").is_none(),
        "a stopped agent cannot be granted a repo"
    );
    h.state_mut()
        .handle_pane(PaneMsg::Text("$ last screen".into()));
    h.run();
    h.get_by_label_contains("$ last screen");
    h.get_by_label("Diff");
}

#[test]
fn a_stopped_agents_panel_offers_no_add_repo() {
    let mut a = app();
    let mut gone = agent("a1", "fix login", Status::Stopped, &["core"]);
    gone.stop_cause = Some(StopCause::Stopped);
    a.handle_stream(snapshot(vec![
        gone,
        agent("a2", "running one", Status::Working, &["core"]),
    ]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("fix login").click();
    h.run();
    assert_eq!(h.state().preview_agent(), Some("a1"));
    assert!(h
        .query_by_role_and_label(Role::Button, "Add repo")
        .is_none());
    h.get_by_label("running one").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Add repo");
}

#[test]
fn add_repo_offers_only_the_agents_product_repos() {
    let mut a = app();
    let owned = |name: &str, product: &str| fleet_api::RepoInfo {
        product: product.into(),
        ..repo_info(name)
    };
    a.handle_result(AppResult::Repos(Ok(vec![
        owned("mine", "p"),
        owned("theirs", "w"),
    ])));
    a.handle_stream(snapshot(vec![agent(
        "a1",
        "fix login",
        Status::Working,
        &[],
    )]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("fix login").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Add repo").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "mine");
    assert!(
        h.query_by_role_and_label(Role::Button, "theirs").is_none(),
        "another product's repo is offered"
    );
}

#[test]
fn the_panel_details_leave_the_terminal_room_at_800_px() {
    let mut a = app();
    let goal =
        "Add pagination to the shots list endpoint, keep the sort order stable across pages and \
                document the new query parameters in the API reference";
    let mut busy = agent("a1", goal, Status::NeedsYou, &["core", "portal"]);
    busy.blocked_reason = Some(fleet_api::BlockedReason::Permission);
    busy.has_changes = true;
    a.handle_stream(snapshot(vec![busy]));
    let mut h = harness(a);
    h.run();
    h.get_by_label(goal).click();
    h.run();
    // The column's title and the panel's status.
    assert_eq!(h.query_all_by_label("Needs you").count(), 2);
    let tabs = h.get_by_role_and_label(Role::Button, "Terminal").rect();
    assert!(
        tabs.bottom() <= 400.0,
        "the terminal starts at y={} of 800",
        tabs.bottom()
    );
}

fn task(
    id: &str,
    state: fleet_api::TaskState,
    cause: Option<StopCause>,
    bytes: u64,
) -> fleet_api::Task {
    fleet_api::Task {
        id: AgentId(id.into()),
        name: format!("proj-{id}"),
        goal: format!("task {id}"),
        workdir: format!("/p/{id}"),
        repos: vec![ws("core")],
        state,
        stop_cause: cause,
        created_at: Some(NOW - 600),
        stopped_at: None,
        archived_at: None,
        agents_used: 1,
        times: fleet_api::StatusTimes::default(),
        log: vec![],
        footprint: Some(fleet_api::Footprint {
            measured_at: NOW,
            items: vec![],
            total: bytes,
        }),
        last_screen: None,
        archive: None,
        persona: "p".into(),
        product: "p".into(),
        release: None,
        breakdowns: vec![],
        gate_runs: vec![],
        prs: vec![],
        done_at: None,
        done_by_hand: false,
        pr_rules: None,
        inbox: vec![],
        kind: fleet_api::AgentKind::Solo,
        lead: None,
        role: None,
        specialists: vec![],
    }
}

/// One running and one stopped task, plus whatever `more` adds.
fn task_list(more: Vec<fleet_api::Task>, orphans: Vec<fleet_api::Orphan>) -> fleet_api::TaskList {
    use fleet_api::TaskState::*;
    let mut tasks = vec![
        task("r1", Running, None, 1_000_000_000),
        task("s1", Stopped, Some(StopCause::Stopped), 2_000_000_000),
        // Archived tasks hold nothing and are not counted.
        task("a1", Archived, Some(StopCause::Stopped), 9_000_000_000),
    ];
    tasks.extend(more);
    fleet_api::TaskList {
        tasks,
        orphans,
        clones_bytes: None,
    }
}

fn orphan() -> fleet_api::Orphan {
    fleet_api::Orphan {
        id: "task_folder:/p/lost".into(),
        kind: fleet_api::OrphanKind::TaskFolder,
        path: "/p/lost".into(),
        worktrees: vec![],
        bytes: Some(500_000_000),
    }
}

#[test]
fn header_counts_show_crashed_and_orphans_only_when_present() {
    let mut a = app();
    a.handle_result(AppResult::Tasks(Ok(task_list(vec![], vec![]))));
    let mut h = harness(a);
    h.run();
    h.get_by_label("Tasks:");
    h.get_by_label("1 running");
    h.get_by_label("1 stopped");
    h.get_by_label("· 3.0 GB");
    assert!(h.query_by_label_contains("crashed").is_none());
    assert!(h.query_by_label_contains("orphan").is_none());

    let crashed = task(
        "c1",
        fleet_api::TaskState::Stopped,
        Some(StopCause::Crashed),
        0,
    );
    h.state_mut().handle_result(AppResult::Tasks(Ok(task_list(
        vec![crashed],
        vec![orphan()],
    ))));
    h.run();
    // Stopped counts every cause; crashed is also shown on its own.
    h.get_by_label("2 stopped");
    h.get_by_label("1 crashed");
    h.get_by_label("1 orphan");
    h.get_by_label("· 3.5 GB");
}

#[test]
fn clicking_stopped_opens_tasks_filtered_to_stopped() {
    let mut a = app();
    a.handle_result(AppResult::Tasks(Ok(task_list(vec![], vec![orphan()]))));
    let mut h = harness(a);
    h.run();
    h.get_by_label("1 stopped").click();
    h.run();
    assert_eq!(h.state().view(), View::Tasks);
    h.get_by_label("Showing stopped");
    h.get_by_role_and_label(Role::Button, "proj-s1");
    assert!(h.query_by_role_and_label(Role::Button, "proj-r1").is_none());

    h.get_by_role_and_label(Role::Button, "Kanban").click();
    h.run();
    h.get_by_label("1 orphan").click();
    h.run();
    h.get_by_label("Showing orphans");
}

#[test]
fn details_on_a_stopped_card_selects_it_in_tasks() {
    let mut a = app();
    let mut gone = agent("s1", "fix login", Status::Stopped, &["core"]);
    gone.stop_cause = Some(StopCause::Stopped);
    a.handle_stream(snapshot(vec![
        gone,
        agent("r1", "running one", Status::Working, &["core"]),
    ]));
    a.handle_result(AppResult::Tasks(Ok(task_list(vec![], vec![]))));
    let mut h = harness(a);
    h.run();
    // Only the stopped card offers it.
    assert_eq!(h.query_all_by_label("Details").count(), 1);
    h.get_by_role_and_label(Role::Button, "Details").click();
    h.run();
    assert_eq!(h.state().view(), View::Tasks);
    assert_eq!(h.state().tasks_selected(), Some("s1"));
    assert!(
        h.query_by_label("Showing stopped").is_none(),
        "Details shows every task"
    );
}

#[test]
fn the_board_polls_the_tasks_list_for_its_counts() {
    let mut h = harness(app());
    h.run();
    assert_eq!(h.state().view(), View::Kanban);
    assert_eq!(h.state().tasks_fetches(), (1, 0));
}

#[test]
fn clicking_the_crash_line_selects_its_card_and_dismisses_it() {
    let (_log, mut a) = logged_app();
    a.handle_stream(snapshot(vec![
        agent("a1", "fix login", Status::Working, &["core"]),
        agent("a2", "other", Status::Working, &["core"]),
    ]));
    let mut h = harness(a);
    h.run();
    h.state_mut().handle_stream(stopped_event(
        2,
        "a1",
        StopCause::Crashed,
        "crashed while running Bash",
    ));
    h.run();
    let text = "fix login crashed: running Bash";
    h.get_by_label(text).click();
    h.run();
    assert_eq!(h.state().preview_agent(), Some("a1"));
    // Sticky warnings stay until dismissed; the click that selects is also that dismissal.
    assert!(h.query_by_label(text).is_none());
}

#[test]
fn a_line_without_an_agent_still_only_dismisses() {
    let (_log, a) = logged_app();
    let mut h = harness(a);
    h.state_mut()
        .handle_result(AppResult::Action(Err("boom".into())));
    h.run();
    h.get_by_label("boom").click();
    h.run();
    assert_eq!(h.state().preview_agent(), None);
    assert_eq!(h.state().view(), View::Kanban);
    assert!(h.query_by_label("boom").is_none());
}

#[test]
fn a_stop_event_fetches_the_counts_again() {
    let mut a = app();
    a.handle_stream(snapshot(vec![agent(
        "a1",
        "fix login",
        Status::Working,
        &["core"],
    )]));
    let mut h = harness(a);
    h.run();
    assert_eq!(h.state().tasks_fetches(), (1, 0));
    h.state_mut().handle_stream(stopped_event(
        2,
        "a1",
        StopCause::Crashed,
        "crashed while running Bash",
    ));
    // The poll's fetch is still out, so the one the crash asks for follows it rather than racing it.
    assert_eq!(h.state().tasks_fetches(), (1, 0));
    h.state_mut()
        .handle_result(AppResult::Tasks(Ok(task_list(vec![], vec![]))));
    assert_eq!(h.state().tasks_fetches(), (2, 0));
}

#[test]
fn header_badges_are_as_tall_as_their_text_not_the_header() {
    // Found in the real window: a badge in the header's centred row grew to the row's full height.
    let mut a = app();
    let crashed = task(
        "c1",
        fleet_api::TaskState::Stopped,
        Some(StopCause::Crashed),
        0,
    );
    a.handle_result(AppResult::Tasks(Ok(task_list(
        vec![crashed],
        vec![orphan()],
    ))));
    let mut h = harness(a);
    h.run();
    let dark = egui::Visuals::default().dark_mode;
    for (label, tone) in [
        ("1 crashed", fleet_api::Tone::Danger),
        ("1 orphan", fleet_api::Tone::Caution),
    ] {
        let text = h.get_by_label(label).rect();
        let badge = frame_around(&h, text.center(), theme::tone_tint(tone, dark));
        assert!(
            badge.height() < text.height() + 6.0,
            "{label}: badge {badge:?} around text {text:?}"
        );
    }
}

fn login(ok: bool) -> Option<fleet_api::LoginState> {
    Some(fleet_api::LoginState {
        ok,
        account: ok.then(|| "ann".to_string()),
        detail: (!ok).then(|| "token expired".to_string()),
        tool: None,
    })
}

/// A ready persona whose logins both work.
fn persona(id: &str, name: &str) -> fleet_api::Persona {
    fleet_api::Persona {
        id: id.into(),
        name: name.into(),
        host: "github.com".into(),
        commit_name: "Ann Example".into(),
        commit_email: "ann@example.invalid".into(),
        github_auth: fleet_api::GithubAuth::Gh,
        claude_dir: format!("/personas/{id}/claude"),
        gh_dir: format!("/personas/{id}/gh"),
        ready: true,
        claude: login(true),
        github: login(true),
        tasks: 0,
        nicknames: Default::default(),
    }
}

fn personas(list: Vec<fleet_api::Persona>) -> AppResult {
    AppResult::Personas(Ok(fleet_api::PersonaList {
        personas: list,
        claude_command: vec!["claude".into()],
        gh_command: "gh".into(),
        tasks_root: "/tasks".into(),
        claude_path: None,
        gh_path: None,
    }))
}

/// "Personal" (p, active: the first ready one) and "Work" (w).
fn two_personas(a: &mut FleetApp) {
    a.handle_result(personas(vec![
        persona("p", "Personal"),
        persona("w", "Work"),
    ]));
    assert_eq!(a.active_persona(), Some("p"));
}

/// Picks `name` in the persona dropdown labelled `dropdown`.
fn pick_persona(h: &mut Harness<'_, FleetApp>, dropdown: &str, name: &str) {
    h.get_by_role_and_label(Role::ComboBox, dropdown).click();
    h.run();
    h.get_by_role_and_label(Role::Button, name).click();
    h.run();
}

#[test]
fn switcher_sets_active_persona_and_saves() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gui.toml");
    let mut a = app().with_settings(path.clone());
    let mut broken = persona("w", "Work");
    broken.github = login(false);
    let mut draft = persona("d", "Draft");
    draft.ready = false;
    a.handle_result(personas(vec![persona("p", "Personal"), broken, draft]));
    let mut h = harness(a);
    h.run();
    h.get_by_role_and_label(Role::ComboBox, "Active persona")
        .click();
    h.run();
    assert!(
        h.query_by_label_contains("Draft").is_none(),
        "a persona that is not ready is offered"
    );
    // The mark is painted; the row says it in its name.
    h.get_by_role_and_label(Role::Button, "Work ⚠").click();
    h.run();
    assert_eq!(h.state().active_persona(), Some("w"));
    assert_eq!(on_disk(&path).active_persona.as_deref(), Some("w"));
}

#[test]
fn new_task_defaults_to_active_persona() {
    let (mut a, hold) = hanging_app();
    two_personas(&mut a);
    let mut h = modal_harness(a);
    h.run();
    h.get_by_role_and_label(Role::Button, "New task").click();
    h.run();
    assert_eq!(h.state().spawn_state().persona, "p");
    pick_persona(&mut h, "Persona", "Work");
    assert_eq!(h.state().spawn_state().persona, "w");
    assert!(
        h.state().spawn_state().open,
        "a pick in the dropdown closed the dialog"
    );
    h.key_press(egui::Key::Escape);
    h.run();
    // Overridden for that one opening only: the next one starts from the active persona again.
    start_an_agent(&mut h);
    assert_eq!(h.state().spawn_state().persona, "p");
    let body = posted_spawn(&hold);
    assert_eq!(body["persona"], "p", "{body}");
    assert_eq!(body["prompt"], "do the thing", "{body}");
}

/// The JSON body of the first `POST /api/agents` the app sends to `hold`. The product lookups a spawn makes first
/// are answered (one product `p`, holding `repo_info("core")`); other calls are accepted and left hanging, so they
/// neither answer nor fail.
fn posted_spawn(hold: &std::net::TcpListener) -> serde_json::Value {
    use std::io::{BufRead, BufReader, Read, Write};
    hold.set_nonblocking(true).unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    let mut held = Vec::new();
    while Instant::now() < end {
        let Ok((stream, _)) = hold.accept() else {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        };
        stream.set_nonblocking(false).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut first = String::new();
        reader.read_line(&mut first).unwrap();
        let answer = if first.starts_with("GET /api/products ") {
            Some(serde_json::json!({"products": [{
                "id": "p", "name": "p", "default_persona": null, "repos": [], "releases": [], "tasks": 0
            }]}))
        } else if first.starts_with("GET /api/repos?product=p ") {
            Some(
                serde_json::to_value(fleet_api::RepoList {
                    repos: vec![repo_info("core")],
                })
                .unwrap(),
            )
        } else {
            None
        };
        if let Some(body) = answer {
            let body = body.to_string();
            let mut stream = stream;
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            continue;
        }
        if !first.starts_with("POST /api/agents ") {
            held.push(stream);
            continue;
        }
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line.trim().is_empty() {
                break;
            }
            if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = v.trim().parse().unwrap();
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        return serde_json::from_slice(&body).unwrap();
    }
    panic!("no spawn was sent");
}

#[test]
fn changing_dialog_persona_reloads_repos() {
    let mut a = app();
    two_personas(&mut a);
    let mut h = modal_harness(a);
    h.run();
    h.get_by_role_and_label(Role::Button, "New task").click();
    h.run();
    assert_eq!(
        h.state().persona_calls().last(),
        Some(&PersonaCall::Repos("p".into()))
    );
    h.state_mut().handle_result(AppResult::PersonaRepos(
        "p".into(),
        Ok(vec![repo_info("core")]),
    ));
    h.run();
    h.get_by_role_and_label(Role::Button, "core").click();
    h.run();
    pick_persona(&mut h, "Persona", "Work");
    assert_eq!(
        h.state().persona_calls().last(),
        Some(&PersonaCall::Repos("w".into()))
    );
    assert!(
        h.query_by_role_and_label(Role::Button, "core").is_none(),
        "Personal's repos are still offered for Work"
    );
    assert!(
        h.state().spawn_state().picker.chosen.is_empty(),
        "a repo remembered by Personal stays picked for Work"
    );
    // A late answer for the persona no longer picked changes nothing.
    h.state_mut().handle_result(AppResult::PersonaRepos(
        "p".into(),
        Ok(vec![repo_info("core")]),
    ));
    h.state_mut().handle_result(AppResult::PersonaRepos(
        "w".into(),
        Ok(vec![repo_info("work-repo")]),
    ));
    h.run();
    assert!(h.query_by_role_and_label(Role::Button, "core").is_none());
    h.get_by_role_and_label(Role::Button, "work-repo");
}

#[test]
fn persona_filter_hides_other_cards() {
    let mut a = app();
    two_personas(&mut a);
    let mut work = agent("b", "work task", Status::Working, &["core"]);
    work.persona = "w".into();
    a.handle_stream(snapshot(vec![
        agent("a", "personal task", Status::Working, &["core"]),
        work,
    ]));
    let mut h = harness(a);
    h.run();
    h.get_by_role_and_label(Role::ComboBox, "Persona filter")
        .click();
    h.run();
    h.get_by_role_and_label(Role::CheckBox, "Work").click();
    h.run();
    assert!(
        h.query_by_label("personal task").is_none(),
        "a card of another persona is shown"
    );
    h.get_by_label("work task");
    assert_eq!(
        h.state().active_persona(),
        Some("p"),
        "the filter is not the active persona"
    );
    h.get_by_role_and_label(Role::CheckBox, "All personas")
        .click();
    h.run();
    h.get_by_label("personal task");
}

#[test]
fn card_tag_only_when_several_personas_visible() {
    let mut a = app();
    two_personas(&mut a);
    let mut work = agent("b", "work task", Status::Working, &["portal"]);
    work.persona = "w".into();
    a.handle_stream(snapshot(vec![
        agent("a", "personal task", Status::Working, &["core"]),
        work,
    ]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("Personal");
    h.get_by_label("Work");
    // Down to one persona's cards, by the repo filter: the tag says nothing a glance does not.
    h.get_by_role_and_label(Role::ComboBox, "Repo filter")
        .click();
    h.run();
    h.get_by_role_and_label(Role::CheckBox, "core").click();
    h.key_press(egui::Key::Escape);
    h.run();
    h.get_by_label("personal task");
    assert!(h.query_by_label("Personal").is_none());
}

/// Agents "p task" under p and "w task" under w.
fn one_task_each() -> StreamMsg {
    let mut work = agent("b", "w task", Status::Working, &["core"]);
    work.persona = "w".into();
    snapshot(vec![agent("a", "p task", Status::Working, &["core"]), work])
}

#[test]
fn personas_with_one_name_stay_apart_in_the_filter_and_the_tags() {
    let mut a = app();
    a.handle_result(personas(vec![persona("p", "Ann"), persona("w", "Ann")]));
    a.handle_stream(one_task_each());
    let mut h = harness(a);
    h.run();
    h.get_by_label("Ann (p)");
    h.get_by_label("Ann (w)");
    h.get_by_role_and_label(Role::ComboBox, "Persona filter")
        .click();
    h.run();
    h.get_by_role_and_label(Role::CheckBox, "Ann (p)");
    h.get_by_role_and_label(Role::CheckBox, "Ann (w)").click();
    h.run();
    assert!(h.query_by_label("p task").is_none());
    h.get_by_label("w task");
    assert_eq!(h.state().persona_filter(), ["w"]);
}

#[test]
fn a_renamed_persona_stays_selected_and_a_deleted_one_is_dropped() {
    let mut a = app();
    two_personas(&mut a);
    a.handle_stream(snapshot(vec![agent(
        "a",
        "p task",
        Status::Working,
        &["core"],
    )]));
    let mut h = harness(a);
    h.run();
    h.get_by_role_and_label(Role::ComboBox, "Persona filter")
        .click();
    h.run();
    h.get_by_role_and_label(Role::CheckBox, "Work").click();
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(h.query_by_label("p task").is_none());
    h.state_mut().handle_result(personas(vec![
        persona("p", "Personal"),
        persona("w", "Job"),
    ]));
    h.run();
    assert_eq!(h.state().persona_filter(), ["w"], "a rename lost the pick");
    assert!(h.query_by_label("p task").is_none());
    // Work had no agents: once deleted it is not offered, so it must not go on hiding cards.
    h.state_mut()
        .handle_result(AppResult::PersonaDeleted("w".into(), Ok(())));
    h.run();
    assert!(h.state().persona_filter().is_empty());
    h.get_by_label("p task");
}

#[test]
fn a_failed_repo_load_shows_in_the_dialog() {
    let mut a = app();
    two_personas(&mut a);
    let mut h = modal_harness(a);
    h.run();
    h.get_by_role_and_label(Role::Button, "New task").click();
    h.run();
    assert_eq!(
        h.state()
            .persona_calls()
            .iter()
            .filter(|c| **c == PersonaCall::Repos("p".into()))
            .count(),
        1,
        "the dialog's repos were asked for more than once"
    );
    h.state_mut()
        .handle_result(AppResult::PersonaRepos("p".into(), Err("boom".into())));
    h.run();
    h.get_by_label("Could not load the repos: boom");
}

#[test]
fn same_named_personas_stay_apart_in_the_switcher() {
    let mut a = app();
    a.handle_result(personas(vec![persona("p", "Ann"), persona("w", "Ann")]));
    let mut h = harness(a);
    h.run();
    pick_persona(&mut h, "Active persona", "Ann (w)");
    assert_eq!(h.state().active_persona(), Some("w"));
}

#[test]
fn same_named_personas_stay_apart_in_the_new_task_dialog() {
    let mut a = app();
    a.handle_result(personas(vec![persona("p", "Ann"), persona("w", "Ann")]));
    let mut h = modal_harness(a);
    h.run();
    h.get_by_role_and_label(Role::Button, "New task").click();
    h.run();
    pick_persona(&mut h, "Persona", "Ann (w)");
    assert_eq!(h.state().spawn_state().persona, "w");
}

fn def(kind: fleet_api::DefKind, name: &str, error: Option<&str>) -> fleet_api::DefInfo {
    fleet_api::DefInfo {
        kind,
        name: name.into(),
        summary: format!("{name} summary"),
        builtin: true,
        overridden: false,
        model: None,
        error: error.map(String::from),
        mode: None,
        effort: None,
    }
}

/// Two usable orchestrator types and a broken one; two usable specialists and a broken one.
fn fleet_defs() -> AppResult {
    use fleet_api::DefKind::{Orchestrator, Specialist};
    AppResult::Definitions(Ok(vec![
        def(Orchestrator, "broken", Some("orchestrators/broken.md: bad")),
        def(Orchestrator, "direct", None),
        def(Orchestrator, "structured", None),
        def(Specialist, "reviewer", None),
        def(Specialist, "rust", None),
        def(Specialist, "stale", Some("specialists/stale.md: bad")),
    ]))
}

fn fleet_repo(name: &str) -> fleet_api::RepoInfo {
    fleet_api::RepoInfo {
        workflow: fleet_api::WorkflowState::Found {
            base: "main".into(),
            steps: vec!["test".into()],
        },
        ..repo_info(name)
    }
}

/// The New task dialog open under Personal, with `repos` as Personal's remembered ones.
fn fleet_dialog(a: FleetApp, repos: Vec<fleet_api::RepoInfo>) -> Harness<'static, FleetApp> {
    let mut a = a;
    a.handle_result(personas(vec![persona("p", "Personal")]));
    a.handle_result(fleet_defs());
    let mut h = modal_harness(a);
    // The app's fonts, as the dialog is drawn with them.
    theme::apply(&h.ctx);
    h.run();
    h.get_by_role_and_label(Role::Button, "New task").click();
    h.run();
    h.state_mut()
        .handle_result(AppResult::PersonaRepos("p".into(), Ok(repos)));
    h.run();
    h
}

fn type_goal(h: &mut Harness<'static, FleetApp>, text: &str) {
    let goal = h.get_by_role(Role::MultilineTextInput);
    goal.click();
    goal.type_text(text);
    h.run();
}

fn start_enabled(h: &Harness<'static, FleetApp>) -> bool {
    !h.get_by_role_and_label(Role::Button, "Start task")
        .accesskit_node()
        .is_disabled()
}

#[test]
fn type_defaults_to_solo_and_lists_valid_orchestrators() {
    let mut h = fleet_dialog(app(), vec![repo_info("core")]);
    assert_eq!(h.state().spawn_state().kind, fleet_api::AgentKind::Solo);
    assert_eq!(h.state().spawn_state().role, None);
    h.get_by_role_and_label(Role::ComboBox, "Type").click();
    h.run();
    for name in ["Solo", "direct", "structured"] {
        h.get_by_role_and_label(Role::Button, name);
    }
    assert!(
        h.query_by_role_and_label(Role::Button, "broken").is_none(),
        "an orchestrator type that does not load is offered"
    );
    h.get_by_role_and_label(Role::Button, "structured").click();
    h.run();
    assert_eq!(
        h.state().spawn_state().kind,
        fleet_api::AgentKind::Orchestrator
    );
    assert_eq!(h.state().spawn_state().role.as_deref(), Some("structured"));
}

#[test]
fn orchestrator_type_limits_to_one_repo() {
    let mut h = fleet_dialog(app(), vec![fleet_repo("core"), fleet_repo("portal")]);
    h.get_by_role_and_label(Role::Button, "core").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "portal").click();
    h.run();
    assert_eq!(h.state().spawn_state().picker.chosen, ["core", "portal"]);
    pick_persona(&mut h, "Type", "direct");
    assert_eq!(h.state().spawn_state().picker.chosen, ["core"]);
    h.get_by_label_contains("portal was dropped");
    // The orchestrator gets every specialist the product does not hide: there is none to pick here.
    assert!(h.query_by_role_and_label(Role::Button, "rust").is_none());

    // One repo: picking another replaces it.
    h.get_by_role_and_label(Role::Button, "portal").click();
    h.run();
    assert_eq!(h.state().spawn_state().picker.chosen, ["portal"]);

    // Back to Solo: several repos again.
    pick_persona(&mut h, "Type", "Solo");
    h.get_by_role_and_label(Role::Button, "core").click();
    h.run();
    assert_eq!(h.state().spawn_state().picker.chosen, ["portal", "core"]);
}

#[test]
fn start_disabled_without_a_repo() {
    let mut h = fleet_dialog(app(), vec![fleet_repo("core")]);
    type_goal(&mut h, "build it");
    pick_persona(&mut h, "Type", "direct");
    assert!(!start_enabled(&h), "an orchestrated task needs a repo");
    h.get_by_role_and_label(Role::Button, "core").click();
    h.run();
    assert!(start_enabled(&h));
    pick_persona(&mut h, "Type", "Solo");
    assert!(start_enabled(&h));
}

#[test]
fn spawn_sends_kind_and_role() {
    let (a, hold) = hanging_app();
    let mut h = fleet_dialog(a, vec![fleet_repo("core")]);
    type_goal(&mut h, "build it");
    pick_persona(&mut h, "Type", "direct");
    h.get_by_role_and_label(Role::Button, "core").click();
    h.run();
    assert!(start_enabled(&h));
    h.get_by_role_and_label(Role::Button, "Start task").click();
    h.run();
    let body = posted_spawn(&hold);
    assert_eq!(body["kind"], "orchestrator", "{body}");
    assert_eq!(body["role"], "direct", "{body}");
    assert!(body.get("roster").is_none(), "{body}");
    // `core` has a URL, so it starts from there.
    assert_eq!(body["product"], "p", "{body}");
    assert_eq!(
        body["repos"],
        serde_json::json!([{"repo": "core", "local": null}]),
        "{body}"
    );
    assert_eq!(body["prompt"], "build it", "{body}");
}

#[test]
fn refetched_definitions_drop_what_no_longer_loads() {
    use fleet_api::DefKind::{Orchestrator, Specialist};
    let mut h = fleet_dialog(app(), vec![fleet_repo("core")]);
    pick_persona(&mut h, "Type", "direct");
    h.get_by_role_and_label(Role::Button, "core").click();
    h.run();
    // A save in Management broke `direct` and `reviewer`.
    h.state_mut().handle_result(AppResult::Definitions(Ok(vec![
        def(Orchestrator, "direct", Some("bad")),
        def(Orchestrator, "structured", None),
        def(Specialist, "rust", None),
        def(Specialist, "reviewer", Some("bad")),
    ])));
    h.run();
    assert_eq!(h.state().spawn_state().role, None);
    assert_eq!(h.state().spawn_state().kind, fleet_api::AgentKind::Solo);
}

/// The clip rect of the last painted text equal to `text`: a popup paints over the page.
fn text_clip(h: &Harness<'_, FleetApp>, text: &str) -> Option<egui::Rect> {
    fn walk(shape: &egui::Shape, text: &str) -> bool {
        match shape {
            egui::Shape::Text(t) => t.galley.text() == text,
            egui::Shape::Vec(v) => v.iter().any(|s| walk(s, text)),
            _ => false,
        }
    }
    h.output()
        .shapes
        .iter()
        .rfind(|s| walk(&s.shape, text))
        .map(|s| s.clip_rect)
}

#[test]
fn the_persona_dropdowns_last_row_is_not_clipped() {
    let names = ["Alpha", "Bravo", "Charlie", "Delta"];
    let mut a = app();
    a.handle_result(personas(
        names
            .iter()
            .map(|n| persona(&n.to_lowercase(), n))
            .collect(),
    ));
    let mut h = harness(a);
    h.run();
    h.get_by_role_and_label(Role::ComboBox, "Active persona")
        .click();
    for _ in 0..5 {
        h.run();
    }
    for name in names {
        let row = h.get_by_role_and_label(Role::Button, name).rect();
        let clip = text_clip(&h, name).expect("the row's name is painted");
        assert!(
            clip.contains_rect(row),
            "{name}: row {row:?} is cut by its clip {clip:?}"
        );
    }
}

#[test]
fn a_snapshot_refetches_the_tasks() {
    let mut a = app();
    assert_eq!(a.tasks_fetches(), (0, 0));
    a.handle_stream(snapshot(vec![]));
    assert_eq!(a.tasks_fetches(), (1, 0));
}

fn agent_of(id: &str, persona: &str) -> Agent {
    let mut a = agent(id, "task", Status::Working, &[]);
    a.persona = persona.into();
    a
}

#[test]
fn an_unknown_persona_refetches_the_list_once() {
    let mut a = app();
    a.handle_result(personas(vec![persona("p", "Personal")]));
    let before = a.personas_fetches();
    a.handle_stream(snapshot(vec![agent_of("a1", "p"), agent_of("a2", "")]));
    assert_eq!(
        a.personas_fetches(),
        before,
        "known and empty ids ask nothing"
    );
    a.handle_stream(snapshot(vec![agent_of("a1", "newbie")]));
    assert_eq!(a.personas_fetches(), before + 1);
    // Still unknown after the answer: not asked about again, by a snapshot or an event.
    a.handle_result(personas(vec![persona("p", "Personal")]));
    a.handle_stream(snapshot(vec![agent_of("a1", "newbie")]));
    a.handle_stream(status_changed(2, "a1", Status::NeedsYou));
    assert_eq!(a.personas_fetches(), before + 1);
    // Another unknown id is a new question.
    a.handle_stream(snapshot(vec![agent_of("a1", "other")]));
    assert_eq!(a.personas_fetches(), before + 2);
}

#[test]
fn an_unknown_persona_is_asked_about_again_after_a_failed_refetch() {
    let mut a = app();
    a.handle_result(personas(vec![persona("p", "Personal")]));
    let before = a.personas_fetches();
    a.handle_stream(snapshot(vec![agent_of("a1", "newbie")]));
    assert_eq!(a.personas_fetches(), before + 1);
    a.handle_result(AppResult::Personas(Err("daemon unreachable".into())));
    // The failed fetch said nothing about newbie: the next look asks again.
    a.handle_stream(status_changed(2, "a1", Status::NeedsYou));
    assert_eq!(a.personas_fetches(), before + 2);
    // An answer that still lacks it settles the question.
    a.handle_result(personas(vec![persona("p", "Personal")]));
    a.handle_stream(status_changed(3, "a1", Status::Working));
    assert_eq!(a.personas_fetches(), before + 2);
}

#[test]
fn the_stopped_count_dot_takes_the_stopped_columns_colour() {
    for tone in [fleet_api::Tone::Muted, fleet_api::Tone::Active] {
        let mut schema = default_ui_schema();
        schema
            .columns
            .iter_mut()
            .find(|c| c.statuses.contains(&Status::Stopped))
            .unwrap()
            .tone = tone;
        let mut a = app();
        a.handle_result(AppResult::Schema(Ok(schema)));
        a.handle_result(AppResult::Tasks(Ok(task_list(vec![], vec![]))));
        let mut h = harness(a);
        h.run();
        let label = h.get_by_label("1 stopped").rect();
        let dark = h.ctx.global_style().visuals.dark_mode;
        let want = theme::tone_color(tone, dark);
        let dot = fill_circle_near(&h, label);
        assert_eq!(dot, Some(want), "{tone:?}");
    }
}

/// The colour of the 3.5 px filled circle painted level with and left of `label`.
fn fill_circle_near(h: &Harness<'_, FleetApp>, label: egui::Rect) -> Option<egui::Color32> {
    fn walk(s: &egui::Shape, label: egui::Rect, out: &mut Option<egui::Color32>) {
        match s {
            egui::Shape::Circle(c)
                if (c.radius - 3.5).abs() < 0.01
                    && c.center.x < label.left()
                    && c.center.x > label.left() - 12.0
                    && (c.center.y - label.center().y).abs() < 4.0 =>
            {
                *out = Some(c.fill)
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, label, out)),
            _ => {}
        }
    }
    let mut out = None;
    h.output()
        .shapes
        .iter()
        .for_each(|s| walk(&s.shape, label, &mut out));
    out
}

fn repo_with(name: &str, workflow: fleet_api::WorkflowState) -> fleet_api::RepoInfo {
    fleet_api::RepoInfo {
        workflow,
        ..repo_info(name)
    }
}

const WORKFLOW_PARAGRAPH: &str =
    "Each repo lists its checks in .fleet/workflow.yml on its base branch.";

#[test]
fn dialog_offers_create_workflow_without_blocking() {
    let missing = fleet_api::WorkflowState::Missing {
        base: "main".into(),
    };
    let mut h = fleet_dialog(app(), vec![repo_with("core", missing)]);
    type_goal(&mut h, "build it");
    pick_persona(&mut h, "Type", "direct");
    h.get_by_role_and_label(Role::Button, "core").click();
    h.run();
    h.get_by_label_contains("core has no .fleet/workflow.yml on main.");
    // The line is the clone's last fetch; creation fetches and refuses authoritatively.
    assert!(start_enabled(&h));
    h.get_by_role_and_label(Role::Button, "Create workflow")
        .click();
    h.run();
    assert_eq!(h.state().view(), fleet_gui::views::View::Management);
    h.get_by_label_contains(WORKFLOW_PARAGRAPH);
    assert!(h
        .query_by_role_and_label(Role::Button, "Start task")
        .is_none());
}

#[test]
fn solo_ignores_a_missing_workflow() {
    let missing = fleet_api::WorkflowState::Missing {
        base: "main".into(),
    };
    let mut h = fleet_dialog(app(), vec![repo_with("core", missing)]);
    type_goal(&mut h, "build it");
    h.get_by_role_and_label(Role::Button, "core").click();
    h.run();
    assert!(h
        .query_by_label_contains("has no .fleet/workflow.yml")
        .is_none());
    assert!(start_enabled(&h));
}

#[test]
fn dialog_shows_an_invalid_or_unreadable_workflow() {
    use fleet_api::WorkflowState::{Invalid, Unreadable};
    for (w, line) in [
        (
            Invalid {
                base: "main".into(),
                error: "bad yaml".into(),
            },
            "core: .fleet/workflow.yml on main is invalid: bad yaml",
        ),
        (
            Unreadable {
                base: "main".into(),
                error: "no origin".into(),
            },
            "core: cannot read .fleet/workflow.yml on main: no origin",
        ),
    ] {
        let mut h = fleet_dialog(app(), vec![repo_with("core", w)]);
        type_goal(&mut h, "build it");
        pick_persona(&mut h, "Type", "direct");
        h.get_by_role_and_label(Role::Button, "core").click();
        h.run();
        h.get_by_label_contains(line);
        h.get_by_role_and_label(Role::Button, "Create workflow");
        assert!(start_enabled(&h));
    }
}

#[test]
fn dialog_lists_the_found_steps() {
    let found = fleet_api::WorkflowState::Found {
        base: "main".into(),
        steps: vec!["fmt".into(), "gui".into()],
    };
    let mut h = fleet_dialog(app(), vec![repo_with("core", found)]);
    type_goal(&mut h, "build it");
    pick_persona(&mut h, "Type", "direct");
    h.get_by_role_and_label(Role::Button, "core").click();
    h.run();
    h.get_by_label_contains("Checks on main: fmt, gui");
    assert!(h
        .query_by_role_and_label(Role::Button, "Create workflow")
        .is_none());
    assert!(start_enabled(&h));
}

#[test]
fn dialog_does_not_block_a_typed_repo() {
    let mut h = fleet_dialog(app(), vec![]);
    type_goal(&mut h, "build it");
    pick_persona(&mut h, "Type", "direct");
    let field = h.get_by_label("Path or git URL");
    field.click();
    field.type_text("https://example.test/x.git");
    h.run();
    h.get_by_role_and_label(Role::Button, "Add").click();
    h.run();
    assert_eq!(h.state().spawn_state().picker.chosen.len(), 1);
    h.get_by_label_contains("The workflow is checked when the task is created.");
    assert!(start_enabled(&h));
}

fn terminal_harness(a: Agent, term: Option<Vec<String>>) -> Harness<'static, FleetApp> {
    let mut app = app().with_terminal(term);
    app.handle_stream(snapshot(vec![a]));
    let mut h = Harness::builder()
        .with_size(Vec2::new(1400.0, 800.0))
        .build_ui_state(|ui, app: &mut FleetApp| app.ui(ui), app);
    h.run();
    h.get_by_label("open me").click();
    h.run();
    h
}

fn xterm() -> Option<Vec<String>> {
    Some(vec!["xterm".into(), "-e".into()])
}

#[test]
fn open_in_terminal_is_disabled_when_stopped() {
    let mut a = agent("a1", "open me", Status::Stopped, &["core"]);
    a.stop_cause = Some(StopCause::Stopped);
    let mut h = terminal_harness(a, xterm());
    h.ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
    let button = h.get_by_role_and_label(Role::Button, "Open in terminal");
    assert!(button.accesskit_node().is_disabled());
    button.hover();
    h.run();
    h.get_by_label("the agent is stopped");
    h.get_by_role_and_label(Role::Button, "Open in terminal")
        .click();
    h.run();
    assert!(h.state().terminal_launches().is_empty());
}

#[test]
fn open_in_terminal_runs_the_attach_command() {
    let mut a = agent("a1", "open me", Status::Working, &["core"]);
    a.attach = Some(
        [
            "/opt/fleet manager/fleet-attach",
            "--url",
            "http://127.0.0.1:7878",
            "a1",
        ]
        .map(String::from)
        .to_vec(),
    );
    let mut h = terminal_harness(a, xterm());
    h.get_by_role_and_label(Role::Button, "Open in terminal")
        .click();
    h.run();
    assert_eq!(
        h.state().terminal_launches(),
        ["'/opt/fleet manager/fleet-attach' --url http://127.0.0.1:7878 a1"]
    );
}

#[test]
fn open_in_terminal_says_why_it_cannot() {
    let mut a = agent("a1", "open me", Status::Working, &["core"]);
    a.attach = Some(vec!["fleet-attach".into(), "a1".into()]);
    let mut h = terminal_harness(a, None);
    h.ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
    let button = h.get_by_role_and_label(Role::Button, "Open in terminal");
    assert!(button.accesskit_node().is_disabled());
    button.hover();
    h.run();
    h.get_by_label("no terminal emulator found");
}

// ---- products ----

fn product(id: &str, name: &str) -> fleet_api::Product {
    fleet_api::Product {
        id: id.into(),
        name: name.into(),
        default_persona: None,
        internal_url: None,
        repos: vec![],
        releases: vec![],
        tasks: 0,
        overrides: vec![],
        hidden_specialists: vec![],
    }
}

fn products(list: Vec<fleet_api::Product>) -> AppResult {
    AppResult::Products(Ok(fleet_api::ProductList { products: list }))
}

fn release(id: i64, version: &str) -> fleet_api::Release {
    fleet_api::Release {
        id,
        product: "p".into(),
        version: version.into(),
        start: "2026-10-01".into(),
        target: "2026-11-01".into(),
        released_at: None,
    }
}

/// An app whose `gui.toml` says `active` was the product last selected.
fn app_with_active_product(active: &str) -> (tempfile::TempDir, FleetApp) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gui.toml");
    fleet_client::config::GuiSettings {
        active_product: Some(active.into()),
        ..Default::default()
    }
    .save(&path)
    .unwrap();
    (dir, app().with_settings(path))
}

fn new_task_tooltip(h: &mut Harness<'_, FleetApp>) {
    h.ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
    h.get_by_role_and_label(Role::Button, "New task").hover();
    h.run();
}

#[test]
fn new_task_disabled_without_products() {
    let mut a = app();
    two_personas(&mut a);
    a.handle_result(products(vec![]));
    let mut h = harness(a);
    h.run();
    assert!(h
        .get_by_role_and_label(Role::Button, "New task")
        .accesskit_node()
        .is_disabled());
    new_task_tooltip(&mut h);
    h.get_by_label("Create a product first");
}

#[test]
fn new_task_disabled_when_saved_product_is_gone() {
    let (_dir, mut a) = app_with_active_product("old");
    two_personas(&mut a);
    a.handle_result(products(vec![product("p", "Portal")]));
    let mut h = harness(a);
    h.run();
    assert_eq!(h.state().active_product(), None);
    assert!(h
        .get_by_role_and_label(Role::Button, "New task")
        .accesskit_node()
        .is_disabled());
    new_task_tooltip(&mut h);
    h.get_by_label("Pick a product");
}

#[test]
fn new_product_creates_and_selects() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gui.toml");
    let mut a = app().with_settings(path.clone());
    two_personas(&mut a);
    a.handle_result(products(vec![product("p", "Portal")]));
    let mut h = modal_harness(a);
    h.run();
    h.get_by_role_and_label(Role::Button, "New product").click();
    h.run();
    let name = h.get_by_role_and_label(Role::TextInput, "Name");
    name.click();
    name.type_text("Core");
    h.run();
    h.get_by_role_and_label(Role::Button, "Save").click();
    h.run();
    assert_eq!(
        h.state().product_calls(),
        [fleet_gui::app::ProductCall::Create(
            fleet_api::NewProduct {
                name: "Core".into(),
                default_persona: None,
                internal_url: None,
            },
            vec![]
        )]
    );
    h.state_mut()
        .handle_result(AppResult::ProductSaved(Ok(product("core", "Core"))));
    h.run();
    assert_eq!(h.state().active_product(), Some("core"));
    assert_eq!(
        h.get_by_role_and_label(Role::ComboBox, "Active product")
            .value()
            .as_deref(),
        Some("Core")
    );
    assert_eq!(on_disk(&path).active_product.as_deref(), Some("core"));
}

#[test]
fn new_product_sends_the_internal_url_when_typed() {
    let (mut a, _hold) = hanging_app();
    a.handle_result(products(vec![product("p", "Portal")]));
    let mut h = modal_harness(a);
    h.run();
    h.get_by_role_and_label(Role::Button, "New product").click();
    h.run();
    for (label, text) in [
        ("Name", "Core"),
        ("Internal repo URL", "https://git.example/notes.git"),
    ] {
        let field = h.get_by_role_and_label(Role::TextInput, label);
        field.click();
        field.type_text(text);
        h.run();
    }
    h.get_by_role_and_label(Role::Button, "Save").click();
    h.run();
    assert_eq!(
        h.state().product_calls(),
        [fleet_gui::app::ProductCall::Create(
            fleet_api::NewProduct {
                name: "Core".into(),
                default_persona: None,
                internal_url: Some("https://git.example/notes.git".into()),
            },
            vec![]
        )]
    );
}

#[test]
fn board_shows_only_active_products_agents() {
    let (_dir, mut a) = app_with_active_product("p");
    a.handle_result(products(vec![
        product("p", "Portal"),
        product("q", "Other"),
    ]));
    let mut mine = agent("a1", "mine", Status::Working, &["core"]);
    mine.product = "p".into();
    let mut theirs = agent("a2", "theirs", Status::Working, &["core"]);
    theirs.product = "q".into();
    a.handle_stream(snapshot(vec![mine, theirs]));
    let mut h = harness(a);
    h.run();
    h.get_by_label("mine");
    assert!(h.query_by_label("theirs").is_none());
}

#[test]
fn release_filter_backlog_hides_assigned_tasks() {
    let (_dir, mut a) = app_with_active_product("p");
    let mut p = product("p", "Portal");
    p.releases = vec![release(1, "1.0")];
    a.handle_result(products(vec![p]));
    let mut assigned = agent("a1", "assigned", Status::Working, &["core"]);
    assigned.product = "p".into();
    let mut loose = agent("a2", "loose", Status::Working, &["core"]);
    loose.product = "p".into();
    a.handle_stream(snapshot(vec![assigned, loose]));
    let mut t1 = task("a1", fleet_api::TaskState::Running, None, 1);
    t1.release = Some(1);
    let t2 = task("a2", fleet_api::TaskState::Running, None, 1);
    a.handle_result(AppResult::Tasks(Ok(fleet_api::TaskList {
        tasks: vec![t1, t2],
        orphans: vec![],
        clones_bytes: None,
    })));
    let mut h = harness(a);
    h.run();
    h.get_by_label("assigned");
    h.get_by_label("loose");
    h.get_by_role_and_label(Role::ComboBox, "Release filter")
        .click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Backlog").click();
    h.run();
    assert!(h.query_by_label("assigned").is_none());
    h.get_by_label("loose");
}

// ---- New task: release and start ----

fn start_repo(name: &str, path: Option<&str>, url: Option<&str>) -> fleet_api::RepoInfo {
    fleet_api::RepoInfo {
        path: path.map(String::from),
        url: url.map(String::from),
        ..repo_info(name)
    }
}

/// An app on product `p` with the listed repos, the dialog open under persona `p` and the repos fetched.
fn new_task_harness(
    app: FleetApp,
    mut product: fleet_api::Product,
    repos: Vec<fleet_api::RepoInfo>,
) -> Harness<'static, FleetApp> {
    let mut app = app;
    product.repos = repos.clone();
    app.handle_result(products(vec![product]));
    let mut h = modal_harness(app);
    h.run();
    h.get_by_role_and_label(Role::Button, "New task").click();
    h.run();
    h.state_mut()
        .handle_result(AppResult::PersonaRepos("p".into(), Ok(repos)));
    h.run();
    h
}

fn pick_repo(h: &mut Harness<'static, FleetApp>, name: &str) {
    h.get_by_role_and_label(Role::Button, name).click();
    h.run();
}

/// A git repo with one commit, so `git worktree list` has a main checkout to report.
fn git_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.invalid",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "init",
        ],
    ] {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
    }
    dir
}

#[test]
fn start_choice_offered_only_with_path_and_url() {
    let (a, _hold) = hanging_app();
    let repos = vec![
        start_repo(
            "both",
            Some("/nowhere/both"),
            Some("https://example.invalid/both.git"),
        ),
        start_repo("web", None, Some("https://example.invalid/web.git")),
    ];
    let mut h = new_task_harness(a, product("p", "p"), repos);
    pick_repo(&mut h, "web");
    assert!(h.query_by_label("From URL").is_none());
    assert!(h.query_by_label("Local checkout").is_none());
    pick_repo(&mut h, "both");
    h.get_by_role_and_label(Role::Button, "From URL");
    h.get_by_role_and_label(Role::Button, "Local checkout");
}

#[test]
fn path_only_repo_shows_no_github_hint() {
    let (a, _hold) = hanging_app();
    let repos = vec![
        start_repo("tool", Some("/nowhere/tool"), None),
        start_repo("web", None, Some("https://example.invalid/web.git")),
    ];
    let mut h = new_task_harness(a, product("p", "p"), repos);
    pick_repo(&mut h, "web");
    let hint = "No URL: no pull request or merge tracking for this repo.";
    assert!(h.query_by_label(hint).is_none());
    pick_repo(&mut h, "tool");
    h.get_by_label(hint);
    assert!(
        h.query_by_label("From URL").is_none(),
        "no choice without a URL"
    );
}

#[test]
fn persona_defaults_to_products_default() {
    let (mut a, _hold) = hanging_app();
    two_personas(&mut a);
    let mut prod = product("p", "p");
    prod.default_persona = Some("w".into());
    a.handle_result(products(vec![prod.clone()]));
    let mut h = modal_harness(a);
    h.run();
    h.get_by_role_and_label(Role::Button, "New task").click();
    h.run();
    assert_eq!(h.state().spawn_state().persona, "w");
    h.key_press(egui::Key::Escape);
    h.run();

    // A default that is not a ready persona falls back to the active one.
    prod.default_persona = Some("gone".into());
    h.state_mut().handle_result(products(vec![prod]));
    h.run();
    h.get_by_role_and_label(Role::Button, "New task").click();
    h.run();
    assert_eq!(h.state().spawn_state().persona, "p");
}

#[test]
fn spawn_sends_product_release_and_local_start() {
    let repo = git_repo();
    let main = fleet_fs::checkouts(repo.path(), &[]).unwrap().remove(0);
    assert!(main.main);
    let (a, hold) = hanging_app();
    let mut prod = product("p", "p");
    let mut released = release(8, "1.1.0");
    released.released_at = Some(1);
    prod.releases = vec![release(7, "1.2.0"), released];
    let repos = vec![start_repo(
        "core",
        Some(repo.path().to_str().unwrap()),
        Some("https://example.invalid/core.git"),
    )];
    let mut h = new_task_harness(a, prod, repos);
    let prompt = h.get_by_role(Role::MultilineTextInput);
    prompt.click();
    prompt.type_text("do the thing");
    h.run();
    h.get_by_role_and_label(Role::ComboBox, "Release").click();
    h.run();
    assert!(
        h.query_by_label_contains("1.1.0").is_none(),
        "a released one is offered"
    );
    h.get_by_role_and_label(Role::Button, "1.2.0").click();
    h.run();
    assert_eq!(h.state().spawn_state().release, Some(7));
    pick_repo(&mut h, "core");
    h.get_by_role_and_label(Role::Button, "Local checkout")
        .click();
    h.run();
    run_until(&mut h, "the listing", |h| !start_disabled(h));
    h.get_by_role_and_label(Role::Button, "Start task").click();
    h.run();
    let body = posted_spawn(&hold);
    assert_eq!(body["product"], "p", "{body}");
    assert_eq!(body["release"], 7, "{body}");
    assert_eq!(
        body["repos"],
        serde_json::json!([{"repo": "core", "local": main.path.to_str().unwrap()}]),
        "{body}"
    );
}

/// Runs frames until `done`, for the checkout listing that lands from the scanner thread.
fn run_until(
    h: &mut Harness<'static, FleetApp>,
    what: &str,
    done: impl Fn(&Harness<'static, FleetApp>) -> bool,
) {
    let end = Instant::now() + Duration::from_secs(10);
    while !done(h) {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
        h.run();
    }
}

fn start_disabled(h: &Harness<'static, FleetApp>) -> bool {
    h.get_by_role_and_label(Role::Button, "Start task")
        .accesskit_node()
        .is_disabled()
}

#[test]
fn a_release_not_in_the_product_is_dropped_when_it_changes() {
    let (a, _hold) = hanging_app();
    let mut prod = product("p", "p");
    prod.releases = vec![release(7, "1.2.0")];
    let mut h = new_task_harness(a, prod, vec![]);
    h.get_by_role_and_label(Role::ComboBox, "Release").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "1.2.0").click();
    h.run();
    assert_eq!(h.state().spawn_state().release, Some(7));
    // The release was marked released (or the product changed): the dropdown says Backlog, so Backlog it is.
    let mut released = release(7, "1.2.0");
    released.released_at = Some(5);
    let mut prod = product("p", "p");
    prod.releases = vec![released];
    h.state_mut().handle_result(products(vec![prod]));
    h.run();
    assert_eq!(h.state().spawn_state().release, None);
}

#[test]
fn refetched_repos_clear_the_picked_starts() {
    let repo = git_repo();
    let (a, _hold) = hanging_app();
    let repos = vec![start_repo(
        "core",
        Some(repo.path().to_str().unwrap()),
        Some("https://example.invalid/core.git"),
    )];
    let mut h = new_task_harness(a, product("p", "p"), repos.clone());
    pick_repo(&mut h, "core");
    h.get_by_role_and_label(Role::Button, "Local checkout")
        .click();
    h.run();
    assert!(h.state().spawn_state().starts["core"].is_some());
    h.state_mut()
        .handle_result(AppResult::PersonaRepos("p".into(), Ok(repos)));
    h.run();
    assert!(h.state().spawn_state().starts.is_empty());
}

#[test]
fn start_waits_for_a_local_start_to_list_and_stops_on_a_failure() {
    let good = git_repo();
    let (a, hold) = hanging_app();
    let repos = vec![
        start_repo("tool", Some("/nowhere/tool"), None),
        start_repo("ok", Some(good.path().to_str().unwrap()), None),
    ];
    let mut h = new_task_harness(a, product("p", "p"), repos);
    let prompt = h.get_by_role(Role::MultilineTextInput);
    prompt.click();
    prompt.type_text("go");
    h.run();
    pick_repo(&mut h, "ok");
    run_until(&mut h, "the listing", |h| !start_disabled(h));
    assert!(!start_disabled(&h), "a listed checkout starts");
    pick_repo(&mut h, "tool");
    run_until(&mut h, "the failure", |h| {
        h.query_by_label_contains("Cannot list the checkouts of tool")
            .is_some()
    });
    assert!(
        start_disabled(&h),
        "a repo that failed to list blocks Start"
    );
    drop(hold);
}

#[test]
fn the_strip_scan_line_runs_only_while_animating() {
    let focus = |h: &mut Harness<'static, FleetApp>, on: bool| {
        h.input_mut()
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .focused = Some(on);
    };
    let delay = |h: &Harness<'static, FleetApp>| {
        h.output().viewport_output[&egui::ViewportId::ROOT].repaint_delay
    };
    let mut h = harness(app());
    h.step();
    let strip = h.state().layout_rects().strip;
    let accent = theme::tokens(h.ctx.global_style().visuals.dark_mode).fill;
    let scans = |h: &Harness<'static, FleetApp>| {
        fn walk(s: &egui::Shape, out: &mut usize, strip: egui::Rect, accent: egui::Color32) {
            match s {
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out, strip, accent)),
                egui::Shape::Mesh(m)
                    if !m.vertices.is_empty()
                        && m.vertices.iter().all(|v| {
                            strip.expand(0.5).contains(v.pos) && v.pos.y >= strip.bottom() - 2.0
                        })
                        && m.vertices.iter().any(|v| v.color == accent) =>
                {
                    *out += 1
                }
                _ => {}
            }
        }
        let mut n = 0;
        h.output()
            .shapes
            .iter()
            .for_each(|s| walk(&s.shape, &mut n, strip, accent));
        n
    };
    assert_eq!(scans(&h), 0, "unfocused");
    focus(&mut h, true);
    h.step();
    assert!(scans(&h) > 0, "focused");
    assert!(
        delay(&h) <= theme::SWEEP_FRAME,
        "the line moves ~25 px per 100 ms frame: {:?}",
        delay(&h)
    );
    theme::set_animations(&h.ctx, false);
    h.step();
    assert_eq!(scans(&h), 0, "animations off");
    assert!(
        delay(&h) > theme::SWEEP_FRAME,
        "no repaint asked: {:?}",
        delay(&h)
    );
    assert!(delay(&h) > theme::ANIMATION_FRAME, "{:?}", delay(&h));
}
