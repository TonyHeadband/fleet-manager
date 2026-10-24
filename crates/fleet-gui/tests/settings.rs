mod common;

use eframe::egui::{self, accesskit::Role, Vec2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use fleet_api::{default_ui_schema, Agent, AgentId, BlockedReason, RepoRequest, Snapshot, Status};
use fleet_client::config;
use fleet_client::daemon_control::Started;
use fleet_client::http::FleetClient;
use fleet_client::stream::StreamMsg;
use fleet_gui::app::{AppResult, FleetApp};
use fleet_gui::shell::{CloseFlow, DaemonCtl, Root};
use fleet_gui::views::{settings::SettingsTab, View};
use fleet_log::Notice;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type Calls = Arc<Mutex<Vec<&'static str>>>;

struct Fx {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    config: PathBuf,
    state: PathBuf,
    calls: Calls,
}

impl Fx {
    fn root(&self) -> PathBuf {
        self.home.join("projects")
    }

    fn calls(&self) -> Vec<&'static str> {
        self.calls.lock().unwrap().clone()
    }
}

fn fx() -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_path_buf();
    let config = home.join(".config/fleet-manager/config.toml");
    config::write_initial(&config, &home.join("projects"), &home).unwrap();
    let state = home.join("state");
    std::fs::create_dir_all(&state).unwrap();
    Fx { _tmp: tmp, home, config, state, calls: Arc::new(Mutex::new(vec![])) }
}

/// A `DaemonCtl` whose stop and ensure only record that they ran; `ensure` answers with `started`.
fn ctl(fx: &Fx, started: impl Fn() -> Result<Started, String> + Send + Sync + 'static) -> DaemonCtl {
    let (stops, ensures) = (fx.calls.clone(), fx.calls.clone());
    DaemonCtl {
        ensure: Arc::new(move || {
            ensures.lock().unwrap().push("ensure");
            started()
        }),
        stop: Arc::new(move || {
            stops.lock().unwrap().push("stop");
            Ok(())
        }),
        config_path: fx.config.clone(),
        state_dir: fx.state.clone(),
        listen: "127.0.0.1:7878".into(),
        we_started: false,
    }
}

fn agent(id: &str, status: Status) -> Agent {
    Agent {
        id: AgentId(id.into()),
        task: format!("task {id}"),
        workdir: "/w".into(),
        workspaces: vec![],
        tmux_session: format!("fleet-{id}"),
        status,
        activity: String::new(),
        status_since: 1000,
        has_changes: false,
        paused: false,
        owner: "antonio".into(),
        blocked_reason: None,
        repo_request: None,
    }
}

/// A detached app on Settings → General, with the log reaching it. Keep the guard alive.
fn settings_app(fx: &Fx, d: DaemonCtl, agents: Vec<Agent>) -> (tracing::subscriber::DefaultGuard, FleetApp) {
    let (guard, handle) = common::log::test_log();
    let mut app = FleetApp::detached(FleetClient::new("http://127.0.0.1:1"), egui::Context::default())
        .with_locations(fx.home.join("gui.toml"), fx.root(), fx.home.clone())
        .with_daemon(d)
        .with_log(handle);
    app.handle_result(AppResult::Schema(Ok(default_ui_schema())));
    app.handle_stream(StreamMsg::Snapshot(Snapshot { seq: 1, agents }));
    app.set_view(View::Settings);
    (guard, app)
}

fn harness(app: FleetApp) -> Harness<'static, FleetApp> {
    Harness::builder().with_max_steps(60).with_size(Vec2::new(1400.0, 800.0)).build_ui_state(|ui, app: &mut FleetApp| app.ui(ui), app)
}

fn has_notice(app: &FleetApp, notice: Notice, text: &str) -> bool {
    app.log_entries().iter().any(|e| e.notice() == Some(notice) && e.message == text)
}

fn wait_for(h: &mut Harness<'_, FleetApp>, mut done: impl FnMut(&FleetApp) -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        h.run();
        if done(h.state()) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// Appends `suffix` to the Projects folder field, so it names a folder other than the current one.
fn type_root(h: &mut Harness<'_, FleetApp>, suffix: &str) {
    let field = h.get_by_role_and_label(Role::TextInput, "Projects folder");
    field.click();
    field.type_text(suffix);
    h.run();
}

fn save_disabled(h: &Harness<'_, FleetApp>) -> bool {
    h.get_by_role_and_label(Role::Button, "Save").accesskit_node().is_disabled()
}

#[test]
fn save_restarts_the_daemon() {
    let fx = fx();
    let (_log, app) = settings_app(&fx, ctl(&fx, || Ok(Started::StartedByUs)), vec![]);
    let mut h = harness(app);
    h.run();
    assert_eq!(h.get_by_role_and_label(Role::TextInput, "Projects folder").value().as_deref(), Some("~/projects"));
    type_root(&mut h, "2");
    h.get_by_label_contains("cd ~/projects2 && claude");
    h.get_by_role_and_label(Role::Button, "Save").click();
    assert!(wait_for(&mut h, |a| has_notice(a, Notice::Success, "Daemon restarted")), "no Daemon restarted notice");
    h.get_by_label("Daemon restarted");
    assert_eq!(fx.calls(), vec!["stop", "ensure"]);
    assert_eq!(config::load(&fx.config, &fx.home).unwrap().projects_root, fx.home.join("projects2"));
    assert!(fx.home.join("projects2").is_dir());
    assert!(h.state_mut().take_restarted());
    assert!(!h.state_mut().take_restarted(), "reported once");
    assert!(save_disabled(&h), "the saved folder is now the current one");
    h.get_by_role_and_label(Role::Button, "Daemon").click();
    h.run();
    h.get_by_label("this window");
}

#[test]
fn save_is_disabled_for_the_current_root() {
    let fx = fx();
    let (_log, app) = settings_app(&fx, ctl(&fx, || Ok(Started::StartedByUs)), vec![]);
    let mut h = harness(app);
    h.run();
    assert!(save_disabled(&h));
    type_root(&mut h, "/");
    assert!(save_disabled(&h), "a trailing slash names the same folder");
    type_root(&mut h, "x");
    assert!(!save_disabled(&h));
}

#[test]
fn save_is_disabled_while_an_agent_is_queued() {
    let fx = fx();
    let (_log, app) = settings_app(&fx, ctl(&fx, || Ok(Started::StartedByUs)), vec![agent("a1", Status::Queued)]);
    let mut h = harness(app);
    h.run();
    type_root(&mut h, "2");
    assert!(save_disabled(&h));
    h.ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
    h.get_by_role_and_label(Role::Button, "Save").hover();
    h.run();
    h.get_by_label("An agent is still starting; wait until it runs");
    h.get_by_role_and_label(Role::Button, "Save").click();
    h.run();
    assert!(fx.calls().is_empty());
    h.state_mut().handle_stream(StreamMsg::Snapshot(Snapshot { seq: 2, agents: vec![agent("a1", Status::Working)] }));
    h.run();
    assert!(!save_disabled(&h));
}

#[test]
fn pending_request_asks_first() {
    let fx = fx();
    let mut waiting = agent("a1", Status::NeedsYou);
    waiting.blocked_reason = Some(BlockedReason::RepoAccess);
    waiting.repo_request = Some(RepoRequest { repo: "portal".into(), why: "login".into() });
    let (_log, app) = settings_app(&fx, ctl(&fx, || Ok(Started::StartedByUs)), vec![waiting]);
    let mut h = harness(app);
    h.run();
    type_root(&mut h, "2");
    h.get_by_role_and_label(Role::Button, "Save").click();
    h.run();
    h.get_by_label_contains("Saving restarts the daemon, which ends that request");
    h.get_by_role_and_label(Role::Button, "Cancel").click();
    h.run();
    assert!(h.query_by_label_contains("interrupted").is_none());
    std::thread::sleep(Duration::from_millis(100));
    h.run();
    assert!(fx.calls().is_empty(), "Cancel restarts nothing");
    assert_eq!(config::load(&fx.config, &fx.home).unwrap().projects_root, fx.root(), "Cancel writes nothing");

    h.get_by_role_and_label(Role::Button, "Save").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Restart anyway").click();
    assert!(wait_for(&mut h, |a| has_notice(a, Notice::Success, "Daemon restarted")));
    assert_eq!(fx.calls(), vec!["stop", "ensure"]);
}

#[test]
fn restart_daemon_with_a_pending_request_asks_first() {
    let fx = fx();
    let mut waiting = agent("a1", Status::NeedsYou);
    waiting.blocked_reason = Some(BlockedReason::RepoAccess);
    waiting.repo_request = Some(RepoRequest { repo: "portal".into(), why: "login".into() });
    let (_log, app) = settings_app(&fx, ctl(&fx, || Ok(Started::StartedByUs)), vec![waiting]);
    let mut h = harness(app);
    h.run();
    h.get_by_role_and_label(Role::Button, "Daemon").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Restart daemon").click();
    h.run();
    h.get_by_label_contains("Restarting the daemon ends that request");
    h.get_by_role_and_label(Role::Button, "Cancel").click();
    h.run();
    assert!(h.query_by_label_contains("interrupted").is_none());
    std::thread::sleep(Duration::from_millis(100));
    h.run();
    assert!(fx.calls().is_empty(), "Cancel restarts nothing");

    h.get_by_role_and_label(Role::Button, "Restart daemon").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Restart anyway").click();
    assert!(wait_for(&mut h, |a| has_notice(a, Notice::Success, "Daemon restarted")));
    assert_eq!(fx.calls(), vec!["stop", "ensure"]);
}

#[test]
fn failed_restart_offers_restart_daemon() {
    let fx = fx();
    let fail = Arc::new(Mutex::new(true));
    let f = fail.clone();
    let ensure = move || if *f.lock().unwrap() { Err("fleet-daemon exited at startup".to_string()) } else { Ok(Started::StartedByUs) };
    let (_log, app) = settings_app(&fx, ctl(&fx, ensure), vec![]);
    let mut h = harness(app);
    h.run();
    type_root(&mut h, "2");
    h.get_by_role_and_label(Role::Button, "Save").click();
    assert!(wait_for(&mut h, |a| has_notice(a, Notice::Error, "fleet-daemon exited at startup")), "no error notice");
    assert_eq!(config::load(&fx.config, &fx.home).unwrap().projects_root, fx.home.join("projects2"), "the config stays written");
    assert!(!h.state_mut().take_restarted());

    *fail.lock().unwrap() = false;
    h.get_by_role_and_label(Role::Button, "Daemon").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Restart daemon").click();
    assert!(wait_for(&mut h, |a| has_notice(a, Notice::Success, "Daemon restarted")));
    assert_eq!(fx.calls(), vec!["stop", "ensure", "stop", "ensure"]);
    assert!(h.state_mut().take_restarted());
}

#[test]
fn a_daemon_without_a_pidfile_keeps_running_and_save_says_so() {
    let fx = fx();
    let mut d = ctl(&fx, || Ok(Started::AlreadyRunning));
    let calls = fx.calls.clone();
    d.stop = Arc::new(move || {
        calls.lock().unwrap().push("stop");
        Err(fleet_client::daemon_control::StopError::NoPidfile.to_string())
    });
    let (_log, app) = settings_app(&fx, d, vec![]);
    let mut h = harness(app);
    h.run();
    type_root(&mut h, "2");
    h.get_by_role_and_label(Role::Button, "Save").click();
    assert!(wait_for(&mut h, |a| has_notice(a, Notice::Info, "Saved. Restart the daemon to use the new folder.")));
    assert_eq!(config::load(&fx.config, &fx.home).unwrap().projects_root, fx.home.join("projects2"));
    assert!(!h.state_mut().take_restarted(), "the daemon that runs is still the old one");
}

#[test]
fn daemon_tab_shows_paths() {
    let fx = fx();
    std::fs::write(fx.state.join("daemon.pid"), "4242\n").unwrap();
    let (_log, mut app) = settings_app(&fx, ctl(&fx, || Ok(Started::StartedByUs)), vec![]);
    app.set_view(View::Kanban);
    let mut h = harness(app);
    h.run();
    h.state_mut().set_view(View::Settings);
    h.run();
    h.get_by_role_and_label(Role::Button, "Daemon").click();
    h.run();
    assert_eq!(h.state().settings_tab(), SettingsTab::Daemon);
    for label in ["Address", "Started by", "State directory", "Config file", "PID", "127.0.0.1:7878", "already running", "4242"] {
        h.get_by_label(label);
    }
    h.get_by_label(&fx.state.display().to_string());
    h.get_by_label(&fx.config.display().to_string());
    std::fs::remove_file(fx.state.join("daemon.pid")).unwrap();
    h.run();
    assert!(h.query_by_label("PID").is_none(), "no pidfile, no PID row");
    h.get_by_role_and_label(Role::Button, "Restart daemon");
}

#[test]
fn a_restart_makes_the_window_own_the_daemon() {
    let fx = fx();
    let (_log, app) = settings_app(&fx, ctl(&fx, || Ok(Started::StartedByUs)), vec![]);
    let flow = CloseFlow::new(fx.home.join("gui.toml"), false, Arc::new(|| Ok(())));
    let root = Root::running(egui::Context::default(), app, flow, false);
    let mut h = Harness::builder().with_max_steps(60).with_size(Vec2::new(1400.0, 800.0)).build_ui_state(|ui, r: &mut Root| r.ui(ui), root);
    h.run();
    h.get_by_role_and_label(Role::Button, "Daemon").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Restart daemon").click();
    let end = Instant::now() + Duration::from_secs(10);
    while !h.state().we_started_daemon() && Instant::now() < end {
        h.run();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(h.state().we_started_daemon());
}

#[test]
fn a_close_flow_told_we_started_stops_the_daemon_without_asking() {
    let fx = fx();
    let stops = Arc::new(Mutex::new(0));
    let s = stops.clone();
    let mut flow = CloseFlow::new(
        fx.home.join("gui.toml"),
        false,
        Arc::new(move || {
            *s.lock().unwrap() += 1;
            Ok(())
        }),
    );
    flow.set_we_started(true);
    // Nothing at risk and a daemon this window started: closing stops it silently instead of asking.
    assert_eq!(flow.request_close(&[agent("a1", Status::Review)]), fleet_gui::shell::FlowStep::KeepOpen);
    let mut h = Harness::builder().with_max_steps(60).build_ui_state(|ui, f: &mut CloseFlow| f.dialog_ui(ui), flow);
    let end = Instant::now() + Duration::from_secs(5);
    while *stops.lock().unwrap() == 0 && Instant::now() < end {
        h.run();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(*stops.lock().unwrap(), 1);
    assert!(h.query_by_label_contains("Keep daemon running").is_none(), "it did not ask");
}

/// `restart_daemon` recognises a missing pidfile by the text a `Stopper` reports. This builds the stopper the way
/// `main` does, around the real `stop_daemon`, so a change to that text or to main's mapping breaks here.
#[cfg(unix)]
#[test]
fn a_real_stopper_without_a_pidfile_goes_on_to_ensure() {
    let state = tempfile::tempdir().unwrap();
    let dir = state.path().to_path_buf();
    let stop: fleet_gui::shell::Stopper = Arc::new(move || fleet_client::daemon_control::stop_daemon(&dir).map_err(|e| e.to_string()));
    let ensure: fleet_gui::shell::EnsureFn = Arc::new(|| Ok(Started::AlreadyRunning));
    assert_eq!(fleet_gui::shell::restart_daemon(&stop, &ensure), Ok(Started::AlreadyRunning));
}

/// The New project picker greys out agents' copies; after a Save those live under the new folder, not the old one.
#[test]
fn new_project_excludes_the_saved_projects_root() {
    let fx = fx();
    let (_log, app) = settings_app(&fx, ctl(&fx, || Ok(Started::StartedByUs)), vec![]);
    let mut h = harness(app);
    h.run();
    type_root(&mut h, "2");
    h.get_by_role_and_label(Role::Button, "Save").click();
    assert!(wait_for(&mut h, |a| has_notice(a, Notice::Success, "Daemon restarted")));
    h.state_mut().set_view(View::Kanban);
    h.run();
    h.get_by_role_and_label(Role::Button, "New project").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Browse…").click();
    let end = Instant::now() + Duration::from_secs(10);
    while h.query_by_role_and_label(Role::Button, "projects2").is_none() {
        assert!(Instant::now() < end, "home was never listed");
        h.run();
        std::thread::sleep(Duration::from_millis(20));
    }
    let home = h.get_by_label("Folder path").value().unwrap_or_default();
    h.get_by_role_and_label(Role::Button, "projects2").click();
    h.run();
    assert_eq!(h.get_by_label("Folder path").value().unwrap_or_default(), home, "the new projects folder is not opened");
}

/// A panic on the restart thread must not leave the window believing a restart is still running.
#[test]
fn a_panicking_restart_thread_still_ends_the_restart() {
    let fx = fx();
    let (_log, app) = settings_app(&fx, ctl(&fx, || panic!("ensure blew up")), vec![]);
    let mut h = harness(app);
    h.run();
    h.get_by_role_and_label(Role::Button, "Daemon").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Restart daemon").click();
    assert!(wait_for(&mut h, |a| has_notice(a, Notice::Error, "restart thread panicked")), "no error notice");
    h.run();
    assert!(!h.get_by_role_and_label(Role::Button, "Restart daemon").accesskit_node().is_disabled(), "the restart is over");
}
