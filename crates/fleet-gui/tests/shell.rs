use eframe::egui::{self, Vec2};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use fleet_api::{Agent, AgentId, Status, Workspace};
use fleet_client::close::Summary;
use fleet_client::config::{CloseChoice, GuiSettings, ThemeChoice};
use fleet_client::daemon_control::Started;
use fleet_gui::components::{close_dialog, DialogResult};
use fleet_gui::app::{AppResult, FleetApp};
use fleet_gui::shell::{CloseFlow, FlowStep, Root};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn agent(status: Status) -> Agent {
    Agent {
        id: AgentId(format!("{status:?}")),
        task: "t".into(),
        workdir: "/w".into(),
        workspaces: vec![Workspace { repo: "core".into(), branch: "b".into(), worktree: "/w/core".into(), base: "x".into(), has_changes: false }],
        tmux_session: "s".into(),
        status,
        activity: String::new(),
        status_since: 1,
        has_changes: false,
        paused: false,
        owner: "o".into(),
        blocked_reason: None,
    }
}

// ---- the dialog ----

struct D {
    remember: bool,
    result: Option<DialogResult>,
}

fn dialog_harness(summary: Summary) -> Harness<'static, D> {
    Harness::builder().with_max_steps(60).with_size(Vec2::new(900.0, 600.0)).build_ui_state(
        move |ui, d: &mut D| {
            if let Some(r) = close_dialog(ui.ctx(), &summary, &mut d.remember) {
                d.result = Some(r);
            }
        },
        D { remember: false, result: None },
    )
}

const SUMMARY: Summary = Summary { running: 3, working: 1, needs_you: 2 };

#[test]
fn dialog_shows_the_numbers_both_choices_and_a_remember_box_with_keep_running_focused() {
    let h = dialog_harness(SUMMARY);
    h.get_by_label_contains("3 agents are running");
    h.get_by_label_contains("2 waiting for you");
    h.get_by_label("Keep the daemon running");
    h.get_by_label("Stop the daemon too");
    h.get_by_label("Remember my choice");
    h.get_by_label("Cancel");
    assert!(h.get_by_label("Keep the daemon running").is_focused(), "the safe choice must be the default");
}

#[test]
fn keep_running_and_stop_report_their_choice() {
    let mut h = dialog_harness(SUMMARY);
    h.get_by_label("Keep the daemon running").click();
    h.run();
    assert_eq!(h.state().result, Some(DialogResult::Choose(CloseChoice::KeepRunning)));
    let mut h = dialog_harness(SUMMARY);
    h.get_by_label("Stop the daemon too").click();
    h.run();
    assert_eq!(h.state().result, Some(DialogResult::Choose(CloseChoice::StopDaemon)));
}

#[test]
fn cancel_and_escape_leave_the_window_open() {
    let mut h = dialog_harness(SUMMARY);
    h.get_by_label("Cancel").click();
    h.run();
    assert_eq!(h.state().result, Some(DialogResult::Cancel));
    let mut h = dialog_harness(SUMMARY);
    h.key_press(egui::Key::Escape);
    h.run();
    assert_eq!(h.state().result, Some(DialogResult::Cancel), "Escape must never mean stop the daemon");
}

#[test]
fn remember_is_reported_with_the_choice() {
    let mut h = dialog_harness(SUMMARY);
    h.get_by_label("Remember my choice").click();
    h.run();
    h.get_by_label("Stop the daemon too").click();
    h.run();
    assert!(h.state().remember);
    assert_eq!(h.state().result, Some(DialogResult::Choose(CloseChoice::StopDaemon)));
}

// ---- the close flow ----

struct Fx {
    _tmp: tempfile::TempDir,
    settings: PathBuf,
    stops: Arc<AtomicUsize>,
}

fn fx() -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let settings = tmp.path().join("gui.toml");
    Fx { _tmp: tmp, settings, stops: Arc::new(AtomicUsize::new(0)) }
}

fn flow(fx: &Fx, we_started: bool) -> CloseFlow {
    let stops = fx.stops.clone();
    CloseFlow::new(
        fx.settings.clone(),
        we_started,
        Arc::new(move || {
            stops.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }),
    )
}

fn drive(flow: &mut CloseFlow, h: &mut Harness<'_, CloseFlow>) -> FlowStep {
    let _ = flow;
    h.run();
    h.state_mut().take_step()
}

fn flow_harness(f: CloseFlow) -> Harness<'static, CloseFlow> {
    Harness::builder().with_max_steps(60).with_size(Vec2::new(900.0, 600.0)).build_ui_state(|ui, f: &mut CloseFlow| f.ui(ui), f)
}

fn wait(h: &mut Harness<'_, CloseFlow>) -> FlowStep {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        h.run();
        let step = h.state_mut().take_step();
        if step == FlowStep::CloseNow || Instant::now() > end {
            return step;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn closing_with_nothing_at_risk_and_a_daemon_we_started_stops_it_silently() {
    let fx = fx();
    let mut h = flow_harness(flow(&fx, true));
    assert_eq!(h.state_mut().request_close(&[agent(Status::Review)]), FlowStep::KeepOpen);
    assert_eq!(wait(&mut h), FlowStep::CloseNow);
    assert_eq!(fx.stops.load(Ordering::SeqCst), 1);
    assert!(h.query_by_label("Keep the daemon running").is_none(), "no dialog when nothing can be missed");
}

#[test]
fn closing_a_daemon_we_did_not_start_asks() {
    let fx = fx();
    let mut h = flow_harness(flow(&fx, false));
    assert_eq!(h.state_mut().request_close(&[agent(Status::Review)]), FlowStep::KeepOpen);
    h.run();
    h.get_by_label("Keep the daemon running");
    assert_eq!(fx.stops.load(Ordering::SeqCst), 0);
}

#[test]
fn agents_at_risk_ask_and_keep_running_closes_without_stopping() {
    let fx = fx();
    let mut h = flow_harness(flow(&fx, true));
    h.state_mut().request_close(&[agent(Status::NeedsYou), agent(Status::Working)]);
    h.run();
    h.get_by_label_contains("2 agents are running");
    h.get_by_label("Keep the daemon running").click();
    assert_eq!(wait(&mut h), FlowStep::CloseNow);
    assert_eq!(fx.stops.load(Ordering::SeqCst), 0);
    assert_eq!(GuiSettings::load(&fx.settings).close_choice, None, "nothing is remembered unless asked");
}

#[test]
fn choosing_stop_stops_the_daemon_then_closes_and_can_be_remembered() {
    let fx = fx();
    let mut h = flow_harness(flow(&fx, false));
    h.state_mut().request_close(&[agent(Status::NeedsYou)]);
    h.run();
    h.get_by_label("Remember my choice").click();
    h.run();
    h.get_by_label("Stop the daemon too").click();
    assert_eq!(wait(&mut h), FlowStep::CloseNow);
    assert_eq!(fx.stops.load(Ordering::SeqCst), 1);
    assert_eq!(GuiSettings::load(&fx.settings).close_choice, Some(CloseChoice::StopDaemon));
}

#[test]
fn remembering_a_close_choice_keeps_the_saved_theme() {
    let fx = fx();
    let mut h = flow_harness(flow(&fx, false));
    // The top bar saves the theme while the flow is alive, so the flow's in-memory copy is stale.
    GuiSettings::update(&fx.settings, |s| s.theme = ThemeChoice::Dark).unwrap();
    h.state_mut().request_close(&[agent(Status::NeedsYou)]);
    h.run();
    h.get_by_label("Remember my choice").click();
    h.run();
    h.get_by_label("Keep the daemon running").click();
    assert_eq!(wait(&mut h), FlowStep::CloseNow);
    let saved = GuiSettings::load(&fx.settings);
    assert_eq!(saved.theme, ThemeChoice::Dark);
    assert_eq!(saved.close_choice, Some(CloseChoice::KeepRunning));
}

#[test]
fn cancel_keeps_the_window_open_and_changes_nothing() {
    let fx = fx();
    let mut h = flow_harness(flow(&fx, true));
    h.state_mut().request_close(&[agent(Status::Working)]);
    h.run();
    h.get_by_label("Cancel").click();
    h.run();
    assert_eq!(h.state_mut().take_step(), FlowStep::KeepOpen);
    assert!(h.query_by_label("Keep the daemon running").is_none(), "the dialog closes on cancel");
    assert_eq!(fx.stops.load(Ordering::SeqCst), 0);
}

#[test]
fn a_remembered_choice_skips_the_dialog_and_is_changeable_from_settings() {
    let fx = fx();
    GuiSettings { close_choice: Some(CloseChoice::StopDaemon), ..Default::default() }.save(&fx.settings).unwrap();
    let mut h = flow_harness(flow(&fx, true));
    h.run();
    h.get_by_label_contains("stop the daemon");
    h.get_by_label("Ask me again").click();
    h.run();
    assert_eq!(GuiSettings::load(&fx.settings).close_choice, None, "the setting is cleared on disk");
    h.state_mut().request_close(&[agent(Status::Working)]);
    h.run();
    h.get_by_label("Keep the daemon running");
}

#[test]
fn a_remembered_keep_running_closes_without_asking() {
    let fx = fx();
    GuiSettings { close_choice: Some(CloseChoice::KeepRunning), ..Default::default() }.save(&fx.settings).unwrap();
    let mut h = flow_harness(flow(&fx, true));
    assert_eq!(h.state_mut().request_close(&[agent(Status::NeedsYou)]), FlowStep::CloseNow);
    assert_eq!(fx.stops.load(Ordering::SeqCst), 0);
}

#[test]
fn a_failed_stop_is_shown_and_the_user_can_close_anyway() {
    let fx = fx();
    let f = CloseFlow::new(fx.settings.clone(), true, Arc::new(|| Err("pid 4 still running after 5s".to_string())));
    let mut h = flow_harness(f);
    h.state_mut().request_close(&[]);
    let end = Instant::now() + Duration::from_secs(5);
    while h.query_by_label_contains("still running after 5s").is_none() && Instant::now() < end {
        h.run();
        std::thread::sleep(Duration::from_millis(20));
    }
    h.get_by_label_contains("still running after 5s");
    assert_eq!(h.state_mut().take_step(), FlowStep::KeepOpen, "a failed stop must not close the window silently");
    h.get_by_label("Close anyway").click();
    assert_eq!(wait(&mut h), FlowStep::CloseNow);
    let _ = drive;
}

#[test]
fn closing_again_while_a_stop_is_running_does_not_start_a_second_stop() {
    let fx = fx();
    let release = Arc::new(Mutex::new(()));
    let hold = release.lock().unwrap();
    let (r2, stops) = (release.clone(), fx.stops.clone());
    let f = CloseFlow::new(
        fx.settings.clone(),
        true,
        Arc::new(move || {
            stops.fetch_add(1, Ordering::SeqCst);
            let _wait = r2.lock().unwrap();
            Ok(())
        }),
    );
    let mut h = flow_harness(f);
    assert_eq!(h.state_mut().request_close(&[]), FlowStep::KeepOpen);
    h.run_steps(3);
    assert_eq!(h.state_mut().request_close(&[]), FlowStep::KeepOpen, "a second close request while stopping must wait");
    assert_eq!(h.state_mut().request_close(&[agent(Status::NeedsYou)]), FlowStep::KeepOpen);
    drop(hold);
    assert_eq!(wait(&mut h), FlowStep::CloseNow);
    assert_eq!(fx.stops.load(Ordering::SeqCst), 1, "two stop threads ran");
}

#[test]
fn closing_again_after_choosing_stop_does_not_bring_the_dialog_back() {
    let fx = fx();
    let release = Arc::new(Mutex::new(()));
    let hold = release.lock().unwrap();
    let r2 = release.clone();
    let f = CloseFlow::new(fx.settings.clone(), false, Arc::new(move || { let _w = r2.lock().unwrap(); Ok(()) }));
    let mut h = flow_harness(f);
    h.state_mut().request_close(&[agent(Status::NeedsYou)]);
    h.run();
    h.get_by_label("Stop the daemon too").click();
    h.run_steps(3);
    h.state_mut().request_close(&[agent(Status::NeedsYou)]);
    h.run_steps(3);
    assert!(h.query_by_label("Keep the daemon running").is_none(), "the dialog reappeared over a daemon that is already stopping");
    drop(hold);
    assert_eq!(wait(&mut h), FlowStep::CloseNow);
}

#[test]
fn a_second_close_request_while_the_dialog_is_open_keeps_it_as_it_is() {
    let fx = fx();
    let mut h = flow_harness(flow(&fx, false));
    h.state_mut().request_close(&[agent(Status::NeedsYou)]);
    h.run();
    h.get_by_label("Remember my choice").click();
    h.run();
    assert_eq!(h.state_mut().request_close(&[agent(Status::NeedsYou)]), FlowStep::KeepOpen);
    h.run();
    h.get_by_label("Stop the daemon too").click();
    assert_eq!(wait(&mut h), FlowStep::CloseNow);
    assert_eq!(GuiSettings::load(&fx.settings).close_choice, Some(CloseChoice::StopDaemon), "the ticked box was lost on re-ask");
}

// ---- the whole window ----

#[test]
fn the_remembered_choice_control_is_inside_the_window_with_the_board_drawn() {
    // With a schema loaded the board's central panel takes the remaining height; a control drawn after it
    // lands below the window and cannot be clicked (found in review).
    let fx = fx();
    GuiSettings { close_choice: Some(CloseChoice::KeepRunning), ..Default::default() }.save(&fx.settings).unwrap();
    let mut app = FleetApp::detached(fleet_client::http::FleetClient::new("http://127.0.0.1:1"), egui::Context::default());
    app.handle_result(AppResult::Schema(Ok(fleet_api::default_ui_schema())));
    app.handle_result(AppResult::Repos(Ok(vec!["core".into()])));
    let root = Root::running(egui::Context::default(), app, flow(&fx, true), true);
    let mut h = root_harness(root);
    h.run();
    h.get_by_label("Working");
    let ask = h.get_by_label("Ask me again");
    assert!(ask.rect().max.y <= 700.0 && ask.rect().min.y >= 0.0, "Ask me again is at {:?}, outside the 700 px window", ask.rect());
    ask.click();
    h.run();
    assert_eq!(GuiSettings::load(&fx.settings).close_choice, None);
}

#[test]
fn remembered_choice_line_sits_at_the_bottom() {
    let fx = fx();
    GuiSettings { close_choice: Some(CloseChoice::KeepRunning), ..Default::default() }.save(&fx.settings).unwrap();
    let mut app = FleetApp::detached(fleet_client::http::FleetClient::new("http://127.0.0.1:1"), egui::Context::default());
    app.handle_result(AppResult::Schema(Ok(fleet_api::default_ui_schema())));
    app.handle_result(AppResult::Repos(Ok(vec!["core".into()])));
    let root = Root::running(egui::Context::default(), app, flow(&fx, true), true);
    let mut h = Harness::builder().with_max_steps(60).with_size(Vec2::new(1000.0, 800.0)).build_ui_state(|ui, r: &mut Root| r.ui(ui), root);
    h.run();
    let ask = h.get_by_label("Ask me again").rect().min.y;
    let queued = h.get_by_label("Queued").rect().min.y;
    assert!(ask > queued, "Ask me again (y {ask}) is above the board (Queued at y {queued})");
    assert!(ask > 700.0, "Ask me again is at y {ask}, not in the footer of the 800 px window");
}

// ---- starting up ----

type Ensure = Arc<dyn Fn() -> Result<Started, String> + Send + Sync>;

fn root(fx: &Fx, ensure: Ensure) -> Root {
    Root::new(egui::Context::default(), "http://127.0.0.1:1".into(), fx.settings.clone(), PathBuf::from("/state"), ensure, Arc::new(|| Ok(())))
}

fn root_harness(r: Root) -> Harness<'static, Root> {
    Harness::builder().with_max_steps(60).with_size(Vec2::new(1000.0, 700.0)).build_ui_state(|ui, r: &mut Root| r.ui(ui), r)
}

#[test]
fn a_launch_failure_shows_the_reason_and_retry_runs_it_again() {
    let fx = fx();
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let ensure: Ensure = Arc::new(move || {
        c.fetch_add(1, Ordering::SeqCst);
        Err("fleet-daemon exited at startup\nlog: /state/daemon.log\ntmux 2.7 is too old".to_string())
    });
    let mut h = root_harness(root(&fx, ensure));
    let end = Instant::now() + Duration::from_secs(5);
    while h.query_by_label_contains("tmux 2.7 is too old").is_none() && Instant::now() < end {
        h.run();
        std::thread::sleep(Duration::from_millis(20));
    }
    h.get_by_label_contains("tmux 2.7 is too old");
    h.get_by_label_contains("/state/daemon.log");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    h.get_by_label("Retry").click();
    h.run();
    let end = Instant::now() + Duration::from_secs(5);
    while calls.load(Ordering::SeqCst) < 2 && Instant::now() < end {
        h.run();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn starting_shows_a_message_and_never_blocks_the_window() {
    let fx = fx();
    let release = Arc::new(Mutex::new(()));
    let guard = release.lock().unwrap();
    let r2 = release.clone();
    let ensure: Ensure = Arc::new(move || {
        let _wait = r2.lock().unwrap();
        Ok(Started::StartedByUs)
    });
    let mut h = root_harness(root(&fx, ensure));
    let t = Instant::now();
    for _ in 0..5 {
        h.step();
    }
    assert!(t.elapsed() < Duration::from_millis(500), "frames blocked on the daemon start: {:?}", t.elapsed());
    h.get_by_label_contains("Starting");
    drop(guard);
}

#[test]
fn a_started_daemon_leads_to_the_board() {
    let fx = fx();
    let ensure: Ensure = Arc::new(|| Ok(Started::AlreadyRunning));
    let mut h = root_harness(root(&fx, ensure));
    let end = Instant::now() + Duration::from_secs(5);
    while !h.state().is_running() && Instant::now() < end {
        h.run();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(h.state().is_running());
    h.get_by_label("fleet-board");
    assert!(!h.state().we_started_daemon());
}

#[test]
fn the_binary_prints_its_version() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fleet-gui")).arg("--version").output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), format!("fleet-gui {}", env!("CARGO_PKG_VERSION")));
}
