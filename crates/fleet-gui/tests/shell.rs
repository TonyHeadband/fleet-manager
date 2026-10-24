use eframe::egui::{self, accesskit::Role, Vec2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use fleet_api::{Agent, AgentId, Status, Workspace};
use fleet_client::close::Summary;
use fleet_client::config::{CloseChoice, GuiSettings, ThemeChoice};
use fleet_client::daemon_control::Started;
use fleet_gui::components::{close_dialog, DialogResult};
use fleet_gui::app::{AppResult, FleetApp};
use fleet_gui::shell::{trust_command, CloseFlow, FlowStep, Root};
mod common;

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// `gui.toml` once every background write asked for so far has landed.
fn on_disk(path: &std::path::Path) -> fleet_client::config::GuiSettings {
    fleet_client::config::SettingsWriter::shared().flush();
    fleet_client::config::GuiSettings::load(path)
}


fn agent(status: Status) -> Agent {
    Agent {
        id: AgentId(format!("{status:?}")),
        task: "t".into(),
        workdir: "/w".into(),
        workspaces: vec![Workspace { repo: "core".into(), branch: "b".into(), worktree: "/w/core".into(), base: "x".into(), from_branch: None, has_changes: false, owned: false }],
        tmux_session: "s".into(),
        status,
        activity: String::new(),
        status_since: 1,
        has_changes: false,
        paused: false,
        owner: "o".into(),
        blocked_reason: None,
        repo_request: None,
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
    config: PathBuf,
    stops: Arc<AtomicUsize>,
}

fn fx() -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let settings = tmp.path().join("gui.toml");
    // An existing config is the normal case; first-run tests remove it.
    let config = tmp.path().join("config.toml");
    std::fs::write(&config, "").unwrap();
    Fx { _tmp: tmp, settings, config, stops: Arc::new(AtomicUsize::new(0)) }
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
    Harness::builder().with_max_steps(60).with_size(Vec2::new(900.0, 600.0)).build_ui_state(|ui, f: &mut CloseFlow| f.dialog_ui(ui), f)
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

/// Ask is a choice in Settings: it asks even when nothing can be missed and this window started the daemon.
#[test]
fn closing_on_ask_with_nothing_at_risk_and_a_daemon_we_started_asks() {
    let fx = fx();
    let mut h = flow_harness(flow(&fx, true));
    assert_eq!(h.state_mut().request_close(&[agent(Status::Review)]), FlowStep::KeepOpen);
    h.run();
    h.get_by_label("Keep the daemon running");
    h.get_by_label_contains("No agents are running");
    assert_eq!(fx.stops.load(Ordering::SeqCst), 0);
}

/// Remembering a choice to stop the daemon closes without the dialog.
fn remember_stop(fx: &Fx) {
    GuiSettings::update(&fx.settings, |s| s.close_choice = Some(CloseChoice::StopDaemon)).unwrap();
}

/// A `state_dir` whose `daemon.pid` names `pid`.
fn pidfile(fx: &Fx, pid: u32) -> PathBuf {
    let dir = fx._tmp.path().join("state");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("daemon.pid"), format!("{pid}\n")).unwrap();
    dir
}

#[test]
fn a_remembered_stop_stops_the_daemon_we_started_without_asking() {
    let fx = fx();
    remember_stop(&fx);
    let dir = pidfile(&fx, 4242);
    let mut flow = flow(&fx, true).owning(dir, 4242);
    assert_eq!(flow.request_close(&[agent(Status::Review)]), FlowStep::KeepOpen);
    let mut h = flow_harness(flow);
    let end = Instant::now() + Duration::from_secs(5);
    while fx.stops.load(Ordering::SeqCst) == 0 && Instant::now() < end {
        h.run();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(fx.stops.load(Ordering::SeqCst), 1);
}

#[test]
fn a_daemon_restarted_by_someone_else_is_not_stopped_without_asking() {
    let fx = fx();
    remember_stop(&fx);
    // This window started pid 4242; daemon.pid now names another daemon.
    let dir = pidfile(&fx, 5151);
    let mut h = flow_harness(flow(&fx, true).owning(dir, 4242));
    assert_eq!(h.state_mut().request_close(&[agent(Status::Review)]), FlowStep::KeepOpen);
    h.run();
    h.get_by_label("Keep the daemon running");
    assert_eq!(fx.stops.load(Ordering::SeqCst), 0);
}

#[test]
fn the_close_dialog_counts_follow_the_agents_while_it_is_open() {
    let fx = fx();
    let mut h = flow_harness(flow(&fx, false));
    assert_eq!(h.state_mut().request_close(&[agent(Status::Working)]), FlowStep::KeepOpen);
    h.run();
    h.get_by_label_contains("1 agent is running");
    h.state_mut().refresh(&[agent(Status::Working), agent(Status::NeedsYou)]);
    h.run();
    h.get_by_label_contains("2 agents are running");
    h.state_mut().refresh(&[]);
    h.run();
    h.get_by_label_contains("No agents are running");
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
    assert_eq!(on_disk(&fx.settings).close_choice, None, "nothing is remembered unless asked");
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
    assert_eq!(on_disk(&fx.settings).close_choice, Some(CloseChoice::StopDaemon));
}

#[test]
fn remembering_a_close_choice_keeps_the_saved_theme() {
    let fx = fx();
    let mut h = flow_harness(flow(&fx, false));
    // Settings saves the theme while the flow is alive, so the flow's in-memory copy is stale.
    GuiSettings::update(&fx.settings, |s| s.theme = ThemeChoice::Dark).unwrap();
    h.state_mut().request_close(&[agent(Status::NeedsYou)]);
    h.run();
    h.get_by_label("Remember my choice").click();
    h.run();
    h.get_by_label("Keep the daemon running").click();
    assert_eq!(wait(&mut h), FlowStep::CloseNow);
    let saved = on_disk(&fx.settings);
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
    remember_stop(&fx);
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
    remember_stop(&fx);
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
    assert_eq!(on_disk(&fx.settings).close_choice, Some(CloseChoice::StopDaemon), "the ticked box was lost on re-ask");
}

// ---- the whole window ----

#[test]
fn close_choice_ask_clears_the_saved_choice() {
    let fx = fx();
    GuiSettings { close_choice: Some(CloseChoice::StopDaemon), ..Default::default() }.save(&fx.settings).unwrap();
    let mut app = FleetApp::detached(fleet_client::http::FleetClient::new("http://127.0.0.1:1"), egui::Context::default()).with_settings(fx.settings.clone());
    app.handle_result(AppResult::Schema(Ok(fleet_api::default_ui_schema())));
    app.handle_result(AppResult::Repos(Ok(vec![repo_info("core")])));
    app.handle_stream(fleet_client::stream::StreamMsg::Snapshot(fleet_api::Snapshot { seq: 1, agents: vec![agent(Status::Working)] }));
    // The flow is built before the choice changes, as in the app: it must read the file again when asked to close.
    let root = Root::running(egui::Context::default(), app, flow(&fx, true), true);
    let mut h = root_harness(root);
    h.run();
    h.get_by_role_and_label(Role::Button, "Settings").click();
    h.run();
    h.get_by_label("Ask").click();
    h.run();
    assert_eq!(on_disk(&fx.settings).close_choice, None, "Ask is not saved as no remembered choice");
    h.input_mut().viewports.get_mut(&egui::ViewportId::ROOT).unwrap().events.push(egui::ViewportEvent::Close);
    h.run();
    h.get_by_label("Keep the daemon running");
    assert_eq!(fx.stops.load(Ordering::SeqCst), 0, "the old remembered Stop daemon was used");
}

#[test]
fn a_close_choice_made_in_settings_is_used_without_asking() {
    let fx = fx();
    let mut app = FleetApp::detached(fleet_client::http::FleetClient::new("http://127.0.0.1:1"), egui::Context::default()).with_settings(fx.settings.clone());
    app.handle_result(AppResult::Schema(Ok(fleet_api::default_ui_schema())));
    app.handle_stream(fleet_client::stream::StreamMsg::Snapshot(fleet_api::Snapshot { seq: 1, agents: vec![agent(Status::NeedsYou)] }));
    let root = Root::running(egui::Context::default(), app, flow(&fx, false), false);
    let mut h = root_harness(root);
    h.run();
    h.get_by_role_and_label(Role::Button, "Settings").click();
    h.run();
    h.get_by_label("Keep daemon running").click();
    h.run();
    assert_eq!(on_disk(&fx.settings).close_choice, Some(CloseChoice::KeepRunning));
    h.input_mut().viewports.get_mut(&egui::ViewportId::ROOT).unwrap().events.push(egui::ViewportEvent::Close);
    h.run();
    assert!(h.query_by_label("Keep the daemon running").is_none(), "the dialog asked although Settings says keep running");
}

/// The close dialog's "Remember my choice" writes the setting while Settings may be on screen: a failed stop and
/// Cancel leave the window open, and General must show the remembered choice, not the one it read on entry.
#[test]
fn a_choice_remembered_by_the_close_dialog_shows_in_settings_at_once() {
    let fx = fx();
    let mut app = FleetApp::detached(fleet_client::http::FleetClient::new("http://127.0.0.1:1"), egui::Context::default()).with_settings(fx.settings.clone());
    app.handle_result(AppResult::Schema(Ok(fleet_api::default_ui_schema())));
    app.handle_stream(fleet_client::stream::StreamMsg::Snapshot(fleet_api::Snapshot { seq: 1, agents: vec![agent(Status::NeedsYou)] }));
    let flow = CloseFlow::new(fx.settings.clone(), false, Arc::new(|| Err("pid 4 still running after 5s".to_string())));
    let mut h = root_harness(Root::running(egui::Context::default(), app, flow, false));
    h.run();
    h.get_by_role_and_label(Role::Button, "Settings").click();
    h.run();
    let stop_chip = |h: &Harness<'_, Root>| h.get_by_label("Stop daemon").accesskit_node().toggled() == Some(egui::accesskit::Toggled::True);
    assert!(!stop_chip(&h));
    h.input_mut().viewports.get_mut(&egui::ViewportId::ROOT).unwrap().events.push(egui::ViewportEvent::Close);
    h.run();
    h.get_by_label("Remember my choice").click();
    h.run();
    h.get_by_label("Stop the daemon too").click();
    let end = Instant::now() + Duration::from_secs(5);
    while h.query_by_label_contains("still running after 5s").is_none() && Instant::now() < end {
        h.run();
        std::thread::sleep(Duration::from_millis(20));
    }
    h.get_by_role_and_label(Role::Button, "Cancel").click();
    h.run();
    assert_eq!(on_disk(&fx.settings).close_choice, Some(CloseChoice::StopDaemon));
    assert!(stop_chip(&h), "Settings still shows the choice it read on entry");
}

// ---- starting up ----

type Ensure = Arc<dyn Fn() -> Result<Started, String> + Send + Sync>;

fn root(fx: &Fx, ensure: Ensure) -> Root {
    // A handle with no subscriber behind it: these tests do not look at the log.
    let (_guard, log) = common::log::test_log();
    Root::new(egui::Context::default(), "http://127.0.0.1:1".into(), fx.settings.clone(), PathBuf::from("/state"), fx.config.clone(), fx._tmp.path().to_path_buf(), ensure, Arc::new(|| Ok(())), log)
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
        Ok(Started::StartedByUs(1))
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
    h.get_by_label(fleet_gui::theme::APP_NAME);
    assert!(!h.state().we_started_daemon());
}

#[test]
fn a_config_broken_after_the_daemon_started_still_leads_to_the_board() {
    let fx = fx();
    let config = fx.config.clone();
    // Broken between the daemon's read and the window's.
    let ensure: Ensure = Arc::new(move || {
        std::fs::write(&config, "listen = ").unwrap();
        Ok(Started::StartedByUs(1))
    });
    let mut h = root_harness(root(&fx, ensure));
    let end = Instant::now() + Duration::from_secs(5);
    while !h.state().is_running() && Instant::now() < end {
        h.run();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(h.state().is_running(), "a config read after the daemon started failed the window");
}

#[test]
fn the_binary_prints_its_version() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fleet-gui")).arg("--version").output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), format!("fleet-gui {}", env!("CARGO_PKG_VERSION")));
}

// ---- first run ----

fn first_run_root(fx: &Fx, ensure: Ensure) -> Root {
    std::fs::remove_file(&fx.config).unwrap();
    root(fx, ensure)
}

fn wait_for(h: &mut Harness<'static, Root>, done: impl Fn(&Root) -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while !done(h.state()) && Instant::now() < end {
        h.run();
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn missing_config_shows_first_run_with_the_default_root() {
    let fx = fx();
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let ensure: Ensure = Arc::new(move || {
        c.fetch_add(1, Ordering::SeqCst);
        Ok(Started::AlreadyRunning)
    });
    let mut h = root_harness(first_run_root(&fx, ensure));
    h.run();
    assert!(h.state().is_first_run());
    h.get_by_label("Welcome to Fleet Manager");
    h.get_by_label_contains("Where should projects live?");
    h.get_by_label("Projects folder");
    h.get_by_label("Continue");
    h.get_by_label("Copy");
    assert_eq!(h.state().first_run_root(), Some("~/fleet-projects"));
    assert_eq!(calls.load(Ordering::SeqCst), 0, "the daemon was started before the folder was chosen");
}

#[test]
fn continue_writes_the_config_and_creates_the_folder_then_starts() {
    let fx = fx();
    let ensure: Ensure = Arc::new(|| Ok(Started::StartedByUs(1)));
    let mut h = root_harness(first_run_root(&fx, ensure));
    h.run();
    h.get_by_label("Continue").click();
    h.run_steps(2);
    let home = fx._tmp.path();
    assert!(home.join("fleet-projects").is_dir());
    let text = std::fs::read_to_string(&fx.config).unwrap();
    assert!(text.contains("projects_root = \"~/fleet-projects\""), "{text}");
    wait_for(&mut h, |r| r.is_running());
    assert!(h.state().is_running());
}

#[test]
fn an_unwritable_root_shows_the_error_and_stays() {
    let fx = fx();
    // A file where the folder's parent should be makes create_dir_all fail whoever runs the test.
    let blocker = fx._tmp.path().join("blocker");
    std::fs::write(&blocker, "x").unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let ensure: Ensure = Arc::new(move || {
        c.fetch_add(1, Ordering::SeqCst);
        Ok(Started::AlreadyRunning)
    });
    let mut r = first_run_root(&fx, ensure);
    r.set_first_run_root(&format!("{}/projects", blocker.display()));
    let mut h = root_harness(r);
    h.run();
    h.get_by_label("Continue").click();
    h.run_steps(2);
    assert!(h.state().is_first_run());
    h.get_by_label_contains("Cannot create");
    assert!(!fx.config.exists(), "a config was written for a root that does not exist");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn existing_config_skips_first_run() {
    let fx = fx();
    let ensure: Ensure = Arc::new(|| Ok(Started::AlreadyRunning));
    let mut h = root_harness(root(&fx, ensure));
    h.run();
    assert!(!h.state().is_first_run());
    assert!(h.query_by_label("Welcome to Fleet Manager").is_none());
}

#[test]
fn the_trust_command_quotes_a_root_with_spaces() {
    assert_eq!(trust_command("~/fleet-projects"), "cd ~/fleet-projects && claude");
    assert_eq!(trust_command("~/my projects"), "cd ~/'my projects' && claude");
    assert_eq!(trust_command("/data/it's here"), "cd '/data/it'\\''s here' && claude");
    assert_eq!(trust_command("~"), "cd ~ && claude");
    assert_eq!(trust_command("/a;b"), "cd '/a;b' && claude");
    assert_eq!(trust_command("  "), "cd ~/fleet-projects && claude");
}

fn repo_info(name: &str) -> fleet_api::RepoInfo {
    fleet_api::RepoInfo { name: name.into(), source: fleet_api::RepoSource::Path, location: format!("/repos/{name}") }
}

/// The trust dialog pre-selects "No, exit", so "accept" or "answer 1" would end the session instead.
#[test]
fn the_trust_hint_names_the_second_choice() {
    let fx = fx();
    let ensure: Ensure = Arc::new(|| Ok(Started::AlreadyRunning));
    let mut h = root_harness(first_run_root(&fx, ensure));
    h.run();
    h.get_by_label("Trust it once for Claude: run this, choose \"Yes, I trust this folder\" (the second option), then /exit.");
    assert!(h.query_by_label_contains("accept").is_none());
    h.get_by_label("cd ~/fleet-projects && claude");
}

/// Seen on the real app: Copy wrapped one letter per line and the hint was cut after "/exit".
#[test]
fn the_trust_hint_and_copy_fit_the_card() {
    for root in ["~/fleet-projects", "/data/a/rather/long/path/to/where/all/of/the/projects/live/for/this/user"] {
        let fx = fx();
        let ensure: Ensure = Arc::new(|| Ok(Started::AlreadyRunning));
        let mut r = first_run_root(&fx, ensure);
        r.set_first_run_root(root);
        let mut h = root_harness(r);
        h.run();
        let input = h.get_by_role(egui::accesskit::Role::TextInput).rect();
        // The field shares its row with Browse…; together they span the card.
        let field = input.union(h.get_by_role_and_label(egui::accesskit::Role::Button, "Browse…").rect());
        let copy = h.get_by_role_and_label(egui::accesskit::Role::Button, "Copy").rect();
        let hint = h.get_by_label_contains("Yes, I trust this folder").rect();
        assert!(copy.width() > copy.height() && copy.height() < input.height() * 1.5, "{root}: Copy is {copy:?}");
        for (name, r) in [("hint", hint), ("Copy", copy)] {
            assert!(r.left() >= field.left() - 1.0 && r.right() <= field.right() + 1.0, "{root}: {name} {r:?} leaves the card {field:?}");
        }
    }
}

#[test]
fn first_run_browse_fills_the_root() {
    let fx = fx();
    let ensure: Ensure = Arc::new(|| Ok(Started::AlreadyRunning));
    // `root` is the picker's home (the `Root` is built with the temp dir as home), so it opens there.
    std::fs::create_dir_all(fx._tmp.path().join("docs")).unwrap();
    let mut h = root_harness(first_run_root(&fx, ensure));
    h.run();
    h.get_by_role_and_label(Role::Button, "Browse…").click();
    h.run();
    h.get_by_label("Choose a folder");
    // The listing is read on a thread: wait for its row before choosing the folder it shows.
    let end = Instant::now() + Duration::from_secs(5);
    while h.query_by_role_and_label(Role::Button, "docs").is_none() && Instant::now() < end {
        h.run();
        std::thread::sleep(Duration::from_millis(20));
    }
    h.get_by_role_and_label(Role::Button, "docs").click();
    let want = fx._tmp.path().join("docs").display().to_string();
    let end = Instant::now() + Duration::from_secs(5);
    while h.get_by_label("Folder path").value().as_deref() != Some(want.as_str()) && Instant::now() < end {
        h.run();
    }
    // Select is enabled once the folder just entered has listed.
    while h.get_by_role_and_label(Role::Button, "Select").accesskit_node().is_disabled() && Instant::now() < end {
        h.run();
        std::thread::sleep(Duration::from_millis(20));
    }
    h.get_by_role_and_label(Role::Button, "Select").click();
    h.run();
    assert_eq!(h.state().first_run_root(), Some(want.as_str()));
    assert!(h.state().is_first_run(), "Browse must not continue past the screen");
}
