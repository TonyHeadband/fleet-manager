mod common;

use eframe::egui::{self, accesskit::Role, Key, Modifiers, Vec2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use fleet_api::{default_ui_schema, Agent, AgentId, BlockedReason, RepoRequest, Snapshot, Status};
use fleet_client::config::{self, Limits};
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
        self.home.join("tasks")
    }

    fn personas(&self) -> PathBuf {
        self.home.join("personas")
    }

    fn calls(&self) -> Vec<&'static str> {
        self.calls.lock().unwrap().clone()
    }
}

fn fx() -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_path_buf();
    let config = fleet_fs::dirs::config_dir(&home).join("config.toml");
    config::write_initial(&config, &home.join("tasks"), &home.join("personas"), &home).unwrap();
    let state = home.join("state");
    std::fs::create_dir_all(&state).unwrap();
    Fx {
        _tmp: tmp,
        home,
        config,
        state,
        calls: Arc::new(Mutex::new(vec![])),
    }
}

/// A `DaemonCtl` whose stop and ensure only record that they ran; `ensure` answers with `started`.
fn ctl(
    fx: &Fx,
    started: impl Fn() -> Result<Started, String> + Send + Sync + 'static,
) -> DaemonCtl {
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
        goal: format!("task {id}"),
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
    }
}

/// A detached app on Settings → General, with the log reaching it. Keep the guard alive.
fn settings_app(
    fx: &Fx,
    d: DaemonCtl,
    agents: Vec<Agent>,
) -> (tracing::subscriber::DefaultGuard, FleetApp) {
    let (guard, handle) = common::log::test_log();
    let mut app = FleetApp::detached(
        FleetClient::new("http://127.0.0.1:1"),
        egui::Context::default(),
    )
    .with_product("p")
    .with_locations(
        fx.home.join("gui.toml"),
        fx.root(),
        fx.personas(),
        fx.home.clone(),
    )
    .with_daemon(d)
    .with_log(handle);
    app.handle_result(AppResult::Schema(Ok(default_ui_schema())));
    // A ready persona, as on any board past the setup: New task is gated on it.
    app.handle_result(AppResult::Personas(Ok(fleet_api::PersonaList {
        personas: vec![ready_persona()],
        claude_command: vec!["claude".into()],
        gh_command: "gh".into(),
        tasks_root: "/tasks".into(),
    })));
    app.handle_stream(StreamMsg::Snapshot(Snapshot { seq: 1, agents }));
    app.set_view(View::Settings);
    (guard, app)
}

fn ready_persona() -> fleet_api::Persona {
    fleet_api::Persona {
        id: "p".into(),
        name: "Work".into(),
        host: "github.com".into(),
        commit_name: "Ann Example".into(),
        commit_email: "ann@example.invalid".into(),
        github_auth: fleet_api::GithubAuth::Gh,
        claude_dir: "/personas/p/claude".into(),
        gh_dir: "/personas/p/gh".into(),
        ready: true,
        claude: None,
        github: None,
        tasks: 0,
    }
}

fn harness(app: FleetApp) -> Harness<'static, FleetApp> {
    Harness::builder()
        .with_max_steps(60)
        .with_size(Vec2::new(1400.0, 800.0))
        .build_ui_state(|ui, app: &mut FleetApp| app.ui(ui), app)
}

fn has_notice(app: &FleetApp, notice: Notice, text: &str) -> bool {
    app.log_entries()
        .iter()
        .any(|e| e.notice() == Some(notice) && e.message == text)
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

/// Appends `suffix` to the Tasks folder field, so it names a folder other than the current one.
fn type_root(h: &mut Harness<'_, FleetApp>, suffix: &str) {
    let field = h.get_by_role_and_label(Role::TextInput, "Tasks folder");
    field.click();
    field.type_text(suffix);
    h.run();
}

fn save_disabled(h: &Harness<'_, FleetApp>) -> bool {
    h.get_by_role_and_label(Role::Button, "Save")
        .accesskit_node()
        .is_disabled()
}

#[test]
fn save_restarts_the_daemon() {
    let fx = fx();
    let (_log, app) = settings_app(&fx, ctl(&fx, || Ok(Started::StartedByUs(1))), vec![]);
    let mut h = harness(app);
    h.run();
    assert_eq!(
        h.get_by_role_and_label(Role::TextInput, "Tasks folder")
            .value()
            .as_deref(),
        Some("~/tasks")
    );
    type_root(&mut h, "2");
    h.get_by_role_and_label(Role::Button, "Save").click();
    assert!(
        wait_for(&mut h, |a| has_notice(
            a,
            Notice::Success,
            "Daemon restarted"
        )),
        "no Daemon restarted notice"
    );
    h.get_by_label("Daemon restarted");
    assert_eq!(fx.calls(), vec!["stop", "ensure"]);
    assert_eq!(
        config::load(&fx.config, &fx.home).unwrap().tasks_root,
        fx.home.join("tasks2")
    );
    assert!(fx.home.join("tasks2").is_dir());
    assert_eq!(h.state_mut().take_restarted(), Some(1));
    assert_eq!(h.state_mut().take_restarted(), None, "reported once");
    assert!(save_disabled(&h), "the saved folder is now the current one");
    h.get_by_role_and_label(Role::Button, "Daemon").click();
    h.run();
    h.get_by_label("this window");
}

#[test]
fn save_is_disabled_for_the_current_root() {
    let fx = fx();
    let (_log, app) = settings_app(&fx, ctl(&fx, || Ok(Started::StartedByUs(1))), vec![]);
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
    let (_log, app) = settings_app(
        &fx,
        ctl(&fx, || Ok(Started::StartedByUs(1))),
        vec![agent("a1", Status::Queued)],
    );
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
    h.state_mut().handle_stream(StreamMsg::Snapshot(Snapshot {
        seq: 2,
        agents: vec![agent("a1", Status::Working)],
    }));
    h.run();
    assert!(!save_disabled(&h));
}

#[test]
fn pending_request_asks_first() {
    let fx = fx();
    let mut waiting = agent("a1", Status::NeedsYou);
    waiting.blocked_reason = Some(BlockedReason::RepoAccess);
    waiting.repo_request = Some(RepoRequest {
        repo: "portal".into(),
        why: "login".into(),
    });
    let (_log, app) = settings_app(&fx, ctl(&fx, || Ok(Started::StartedByUs(1))), vec![waiting]);
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
    assert_eq!(
        config::load(&fx.config, &fx.home).unwrap().tasks_root,
        fx.root(),
        "Cancel writes nothing"
    );

    h.get_by_role_and_label(Role::Button, "Save").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Restart anyway")
        .click();
    assert!(wait_for(&mut h, |a| has_notice(
        a,
        Notice::Success,
        "Daemon restarted"
    )));
    assert_eq!(fx.calls(), vec!["stop", "ensure"]);
}

#[test]
fn restart_daemon_with_a_pending_request_asks_first() {
    let fx = fx();
    let mut waiting = agent("a1", Status::NeedsYou);
    waiting.blocked_reason = Some(BlockedReason::RepoAccess);
    waiting.repo_request = Some(RepoRequest {
        repo: "portal".into(),
        why: "login".into(),
    });
    let (_log, app) = settings_app(&fx, ctl(&fx, || Ok(Started::StartedByUs(1))), vec![waiting]);
    let mut h = harness(app);
    h.run();
    h.get_by_role_and_label(Role::Button, "Daemon").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Restart daemon")
        .click();
    h.run();
    h.get_by_label_contains("Restarting the daemon ends that request");
    h.get_by_role_and_label(Role::Button, "Cancel").click();
    h.run();
    assert!(h.query_by_label_contains("interrupted").is_none());
    std::thread::sleep(Duration::from_millis(100));
    h.run();
    assert!(fx.calls().is_empty(), "Cancel restarts nothing");

    h.get_by_role_and_label(Role::Button, "Restart daemon")
        .click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Restart anyway")
        .click();
    assert!(wait_for(&mut h, |a| has_notice(
        a,
        Notice::Success,
        "Daemon restarted"
    )));
    assert_eq!(fx.calls(), vec!["stop", "ensure"]);
}

#[test]
fn failed_restart_offers_restart_daemon() {
    let fx = fx();
    let fail = Arc::new(Mutex::new(true));
    let f = fail.clone();
    let ensure = move || {
        if *f.lock().unwrap() {
            Err("fleet-daemon exited at startup".to_string())
        } else {
            Ok(Started::StartedByUs(1))
        }
    };
    let (_log, app) = settings_app(&fx, ctl(&fx, ensure), vec![]);
    let mut h = harness(app);
    h.run();
    type_root(&mut h, "2");
    h.get_by_role_and_label(Role::Button, "Save").click();
    assert!(
        wait_for(&mut h, |a| has_notice(
            a,
            Notice::Error,
            "fleet-daemon exited at startup"
        )),
        "no error notice"
    );
    assert_eq!(
        config::load(&fx.config, &fx.home).unwrap().tasks_root,
        fx.home.join("tasks2"),
        "the config stays written"
    );
    assert_eq!(h.state_mut().take_restarted(), None);

    *fail.lock().unwrap() = false;
    h.get_by_role_and_label(Role::Button, "Daemon").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Restart daemon")
        .click();
    assert!(wait_for(&mut h, |a| has_notice(
        a,
        Notice::Success,
        "Daemon restarted"
    )));
    assert_eq!(fx.calls(), vec!["stop", "ensure", "stop", "ensure"]);
    assert_eq!(h.state_mut().take_restarted(), Some(1));
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
    assert!(wait_for(&mut h, |a| has_notice(
        a,
        Notice::Info,
        "Saved. Restart the daemon to use the new folder."
    )));
    assert_eq!(
        config::load(&fx.config, &fx.home).unwrap().tasks_root,
        fx.home.join("tasks2")
    );
    assert_eq!(
        h.state_mut().take_restarted(),
        None,
        "the daemon that runs is still the old one"
    );
}

#[test]
fn daemon_tab_shows_paths() {
    let fx = fx();
    std::fs::write(fx.state.join("daemon.pid"), "4242\n").unwrap();
    let (_log, mut app) = settings_app(&fx, ctl(&fx, || Ok(Started::StartedByUs(1))), vec![]);
    app.set_view(View::Kanban);
    let mut h = harness(app);
    h.run();
    h.state_mut().set_view(View::Settings);
    h.run();
    h.get_by_role_and_label(Role::Button, "Daemon").click();
    h.run();
    assert_eq!(h.state().settings_tab(), SettingsTab::Daemon);
    for label in [
        "Address",
        "Started by",
        "State directory",
        "Config file",
        "PID",
        "127.0.0.1:7878",
        "already running",
        "4242",
    ] {
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
    let (_log, app) = settings_app(&fx, ctl(&fx, || Ok(Started::StartedByUs(1))), vec![]);
    let flow = CloseFlow::new(fx.home.join("gui.toml"), false, Arc::new(|| Ok(())));
    let root = Root::running(egui::Context::default(), app, flow, false);
    let mut h = Harness::builder()
        .with_max_steps(60)
        .with_size(Vec2::new(1400.0, 800.0))
        .build_ui_state(|ui, r: &mut Root| r.ui(ui), root);
    h.run();
    h.get_by_role_and_label(Role::Button, "Daemon").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Restart daemon")
        .click();
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
    fleet_client::config::GuiSettings::update(&fx.home.join("gui.toml"), |s| {
        s.close_choice = Some(fleet_client::config::CloseChoice::StopDaemon)
    })
    .unwrap();
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
    let state = fx.home.join("state-restarted");
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(state.join("daemon.pid"), "4242\n").unwrap();
    flow.set_started(state, 4242);
    // A remembered Stop covers only a daemon this window started; after a restart, that is this one.
    assert_eq!(
        flow.request_close(&[agent("a1", Status::Review)]),
        fleet_gui::shell::FlowStep::KeepOpen
    );
    let mut h = Harness::builder()
        .with_max_steps(60)
        .build_ui_state(|ui, f: &mut CloseFlow| f.dialog_ui(ui), flow);
    let end = Instant::now() + Duration::from_secs(5);
    while *stops.lock().unwrap() == 0 && Instant::now() < end {
        h.run();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(*stops.lock().unwrap(), 1);
    assert!(
        h.query_by_label_contains("Keep daemon running").is_none(),
        "it did not ask"
    );
}

/// `restart_daemon` recognises a missing pidfile by the text a `Stopper` reports. This builds the stopper the way
/// `main` does, around the real `stop_daemon`, so a change to that text or to main's mapping breaks here.
#[cfg(unix)]
#[test]
fn a_real_stopper_without_a_pidfile_goes_on_to_ensure() {
    let state = tempfile::tempdir().unwrap();
    let dir = state.path().to_path_buf();
    let stop: fleet_gui::shell::Stopper = Arc::new(move || {
        fleet_client::daemon_control::stop_daemon(&dir).map_err(|e| e.to_string())
    });
    let ensure: fleet_gui::shell::EnsureFn = Arc::new(|| Ok(Started::AlreadyRunning));
    assert_eq!(
        fleet_gui::shell::restart_daemon(&stop, &ensure),
        Ok(Started::AlreadyRunning)
    );
}

/// The New task picker greys out agents' copies; after a Save those live under the new folder, not the old one.
#[test]
fn new_task_excludes_the_saved_tasks_root() {
    let fx = fx();
    let (_log, app) = settings_app(&fx, ctl(&fx, || Ok(Started::StartedByUs(1))), vec![]);
    let mut h = harness(app);
    h.run();
    type_root(&mut h, "2");
    h.get_by_role_and_label(Role::Button, "Save").click();
    assert!(wait_for(&mut h, |a| has_notice(
        a,
        Notice::Success,
        "Daemon restarted"
    )));
    h.state_mut().set_view(View::Kanban);
    h.run();
    h.get_by_role_and_label(Role::Button, "New task").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Browse…").click();
    let end = Instant::now() + Duration::from_secs(10);
    while h.query_by_role_and_label(Role::Button, "tasks2").is_none() {
        assert!(Instant::now() < end, "home was never listed");
        h.run();
        std::thread::sleep(Duration::from_millis(20));
    }
    let home = h.get_by_label("Folder path").value().unwrap_or_default();
    h.get_by_role_and_label(Role::Button, "tasks2").click();
    h.run();
    assert_eq!(
        h.get_by_label("Folder path").value().unwrap_or_default(),
        home,
        "the new tasks folder is not opened"
    );
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
    h.get_by_role_and_label(Role::Button, "Restart daemon")
        .click();
    assert!(
        wait_for(&mut h, |a| has_notice(
            a,
            Notice::Error,
            "restart thread panicked"
        )),
        "no error notice"
    );
    h.run();
    assert!(
        !h.get_by_role_and_label(Role::Button, "Restart daemon")
            .accesskit_node()
            .is_disabled(),
        "the restart is over"
    );
}

/// Text typed into the field while Save is writing the config is the user's next edit, not to be overwritten
/// by the folder just saved. The config is a FIFO here, so the write waits until the test has typed.
#[cfg(unix)]
#[test]
fn an_edit_made_while_saving_survives_the_save() {
    let fx = fx();
    let original = std::fs::read_to_string(&fx.config).unwrap();
    std::fs::remove_file(&fx.config).unwrap();
    assert!(std::process::Command::new("mkfifo")
        .arg(&fx.config)
        .status()
        .unwrap()
        .success());
    let (_log, app) = settings_app(&fx, ctl(&fx, || Ok(Started::StartedByUs(1))), vec![]);
    let mut h = harness(app);
    h.run();
    type_root(&mut h, "2");
    h.get_by_role_and_label(Role::Button, "Save").click();
    h.run();
    type_root(&mut h, "3");
    std::fs::write(&fx.config, original).unwrap();
    assert!(wait_for(&mut h, |a| has_notice(
        a,
        Notice::Success,
        "Daemon restarted"
    )));
    assert_eq!(
        config::load(&fx.config, &fx.home).unwrap().tasks_root,
        fx.home.join("tasks2")
    );
    assert_eq!(
        h.get_by_role_and_label(Role::TextInput, "Tasks folder")
            .value()
            .as_deref(),
        Some("~/tasks23")
    );
}

/// A restart's startup sweep would remove a clone started meanwhile: New task waits for it, and says why.
#[test]
fn new_task_is_blocked_while_the_daemon_restarts() {
    let fx = fx();
    let release = Arc::new(Mutex::new(()));
    let hold = release.lock().unwrap();
    let mut d = ctl(&fx, || Ok(Started::StartedByUs(1)));
    let r = release.clone();
    d.stop = Arc::new(move || {
        let _wait = r.lock().unwrap();
        Ok(())
    });
    let (_log, app) = settings_app(&fx, d, vec![]);
    let mut h = harness(app);
    h.run();
    h.get_by_role_and_label(Role::Button, "Daemon").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Restart daemon")
        .click();
    h.run();
    h.state_mut().set_view(View::Kanban);
    h.run();
    let new_task = || h.get_by_role_and_label(Role::Button, "New task");
    assert!(new_task().accesskit_node().is_disabled());
    h.ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
    new_task().hover();
    h.run();
    h.get_by_label("The daemon is restarting");
    drop(hold);
    assert!(wait_for(&mut h, |a| has_notice(
        a,
        Notice::Success,
        "Daemon restarted"
    )));
    assert!(!h
        .get_by_role_and_label(Role::Button, "New task")
        .accesskit_node()
        .is_disabled());
}

/// Like New task, the Grant box's Browse… greys out agents' copies, which after a Save live under the new folder.
/// The box is opened before the Save, so its picker already exists with the old exclusion.
#[test]
fn the_grant_picker_excludes_the_saved_tasks_root() {
    let fx = fx();
    let mut waiting = agent("a1", Status::NeedsYou);
    waiting.blocked_reason = Some(BlockedReason::RepoAccess);
    waiting.repo_request = Some(RepoRequest {
        repo: "portal".into(),
        why: "login".into(),
    });
    let (_log, mut app) =
        settings_app(&fx, ctl(&fx, || Ok(Started::StartedByUs(1))), vec![waiting]);
    app.set_view(View::Kanban);
    let mut h = harness(app);
    h.run();
    h.get_by_label("task a1").click();
    h.run();
    h.get_by_label("Wants portal");
    h.state_mut().set_view(View::Settings);
    h.run();
    type_root(&mut h, "2");
    h.get_by_role_and_label(Role::Button, "Save").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Restart anyway")
        .click();
    assert!(wait_for(&mut h, |a| has_notice(
        a,
        Notice::Success,
        "Daemon restarted"
    )));
    h.state_mut().set_view(View::Kanban);
    h.run();
    h.get_by_role_and_label(Role::Button, "Browse…").click();
    let end = Instant::now() + Duration::from_secs(10);
    while h.query_by_role_and_label(Role::Button, "tasks2").is_none() {
        assert!(Instant::now() < end, "home was never listed");
        h.run();
        std::thread::sleep(Duration::from_millis(20));
    }
    let home = h.get_by_label("Folder path").value().unwrap_or_default();
    h.get_by_role_and_label(Role::Button, "tasks2").click();
    h.run();
    assert_eq!(
        h.get_by_label("Folder path").value().unwrap_or_default(),
        home,
        "the new tasks folder is not opened"
    );
}

/// What can be in flight when the user turns to Settings: each blocks Save and Restart daemon with its reason.
#[derive(Clone, Copy, Debug)]
enum InFlight {
    Spawn,
    Add,
    Decide,
}

/// Starts `what` against a daemon that accepts and never answers, so it stays in flight, then shows Settings.
fn with_a_call_in_flight(
    fx: &Fx,
    what: InFlight,
) -> (
    tracing::subscriber::DefaultGuard,
    Harness<'static, FleetApp>,
    std::net::TcpListener,
) {
    let hold = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let (guard, handle) = common::log::test_log();
    let mut app = FleetApp::detached(
        FleetClient::new(&format!("http://{}", hold.local_addr().unwrap())),
        egui::Context::default(),
    )
    .with_product("p")
    .with_locations(
        fx.home.join("gui.toml"),
        fx.root(),
        fx.personas(),
        fx.home.clone(),
    )
    .with_daemon(ctl(fx, || Ok(Started::StartedByUs(1))))
    .with_log(handle);
    app.handle_result(AppResult::Schema(Ok(default_ui_schema())));
    let core = fleet_api::RepoInfo {
        name: "core".into(),
        product: "p".into(),
        path: Some("/repos/core".into()),
        url: None,
        roster: vec![],
        workflow: fleet_api::WorkflowState::Missing {
            base: "main".into(),
        },
    };
    app.handle_result(AppResult::Repos(Ok(vec![core])));
    app.handle_result(AppResult::Personas(Ok(fleet_api::PersonaList {
        personas: vec![ready_persona()],
        claude_command: vec!["claude".into()],
        gh_command: "gh".into(),
        tasks_root: "/tasks".into(),
    })));
    let mut a1 = agent("a1", Status::NeedsYou);
    if matches!(what, InFlight::Decide) {
        a1.blocked_reason = Some(BlockedReason::RepoAccess);
        a1.repo_request = Some(RepoRequest {
            repo: "portal".into(),
            why: "login".into(),
        });
    }
    app.handle_stream(StreamMsg::Snapshot(Snapshot {
        seq: 1,
        agents: vec![a1],
    }));
    let mut h = harness(app);
    h.run();
    match what {
        InFlight::Spawn => {
            h.get_by_role_and_label(Role::Button, "New task").click();
            h.run();
            let prompt = h.get_by_role(Role::MultilineTextInput);
            prompt.click();
            prompt.type_text("do the thing");
            h.run();
            h.get_by_role_and_label(Role::Button, "Start task").click();
            h.run();
            assert!(h.state().spawn_state().busy);
            h.key_press(egui::Key::Escape);
            h.run();
        }
        InFlight::Add => {
            h.get_by_label("task a1").click();
            h.run();
            h.get_by_role_and_label(Role::Button, "Add repo").click();
            h.run();
            h.get_by_role_and_label(Role::Button, "core").click();
            h.run();
            h.get_by_role_and_label(Role::Button, "Add repos").click();
            h.run();
        }
        InFlight::Decide => {
            h.get_by_label("task a1").click();
            h.run();
            h.get_by_role_and_label(Role::Button, "Refuse").click();
            h.run();
            assert!(h.state().deciding("a1"));
        }
    }
    h.state_mut().set_view(View::Settings);
    h.run();
    (guard, h, hold)
}

#[test]
fn a_call_in_flight_blocks_save_and_restart_daemon_with_its_reason() {
    for (what, reason) in [
        (InFlight::Spawn, "A task is being started"),
        (InFlight::Add, "A repo is being added"),
        (InFlight::Decide, "A repo request is being answered"),
    ] {
        let fx = fx();
        let (_log, mut h, _hold) = with_a_call_in_flight(&fx, what);
        h.ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
        type_root(&mut h, "2");
        assert!(save_disabled(&h), "{what:?}: Save");
        h.get_by_role_and_label(Role::Button, "Save").hover();
        h.run();
        h.get_by_label(reason);
        h.get_by_role_and_label(Role::Button, "Daemon").click();
        h.run();
        let restart = h.get_by_role_and_label(Role::Button, "Restart daemon");
        assert!(
            restart.accesskit_node().is_disabled(),
            "{what:?}: Restart daemon"
        );
        restart.hover();
        h.run();
        h.get_by_label(reason);
        assert!(fx.calls().is_empty(), "{what:?}: nothing was restarted");
    }
}

// ---- Personas folder ----

const SAVE_PERSONAS: &str = "Save personas folder";
const PERSONAS_NOTE: &str = "Saving restarts the daemon. Existing personas' folders, with their Claude and GitHub logins, are not moved: move them by hand or log the personas in again.";

/// Appends `suffix` to the Personas folder field.
fn type_personas(h: &mut Harness<'_, FleetApp>, suffix: &str) {
    let field = h.get_by_role_and_label(Role::TextInput, "Personas folder");
    field.click();
    field.type_text(suffix);
    h.run();
}

fn save_personas_disabled(h: &Harness<'_, FleetApp>) -> bool {
    h.get_by_role_and_label(Role::Button, SAVE_PERSONAS)
        .accesskit_node()
        .is_disabled()
}

#[test]
fn personas_save_restarts_the_daemon() {
    let fx = fx();
    let (_log, app) = settings_app(&fx, ctl(&fx, || Ok(Started::StartedByUs(1))), vec![]);
    let mut h = harness(app);
    h.run();
    assert_eq!(
        h.get_by_role_and_label(Role::TextInput, "Personas folder")
            .value()
            .as_deref(),
        Some("~/personas")
    );
    type_personas(&mut h, "2");
    h.get_by_role_and_label(Role::Button, SAVE_PERSONAS).click();
    assert!(
        wait_for(&mut h, |a| has_notice(
            a,
            Notice::Success,
            "Daemon restarted"
        )),
        "no Daemon restarted notice"
    );
    assert_eq!(fx.calls(), vec!["stop", "ensure"]);
    let text = std::fs::read_to_string(&fx.config).unwrap();
    assert!(text.contains("personas_dir = \"~/personas2\""), "{text}");
    let saved = config::load(&fx.config, &fx.home).unwrap();
    assert_eq!(saved.personas_dir, fx.home.join("personas2"));
    assert_eq!(saved.tasks_root, fx.root(), "the tasks folder is kept");
    assert!(
        !fx.home.join("personas2").exists(),
        "the daemon creates the folder"
    );
    assert!(
        save_personas_disabled(&h),
        "the saved folder is now the current one"
    );
    assert!(save_disabled(&h), "the tasks folder was not touched");
}

#[test]
fn personas_save_is_disabled_for_the_current_folder() {
    let fx = fx();
    let (_log, app) = settings_app(&fx, ctl(&fx, || Ok(Started::StartedByUs(1))), vec![]);
    let mut h = harness(app);
    h.run();
    assert!(save_personas_disabled(&h));
    h.ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
    h.get_by_role_and_label(Role::Button, SAVE_PERSONAS).hover();
    h.run();
    h.get_by_label("This is the current personas folder");
    type_personas(&mut h, "/");
    assert!(
        save_personas_disabled(&h),
        "a trailing slash names the same folder"
    );
    type_personas(&mut h, "x");
    assert!(!save_personas_disabled(&h));
    assert!(save_disabled(&h), "each field has its own Save");
}

#[test]
fn personas_save_is_disabled_while_an_agent_is_queued() {
    let fx = fx();
    let (_log, app) = settings_app(
        &fx,
        ctl(&fx, || Ok(Started::StartedByUs(1))),
        vec![agent("a1", Status::Queued)],
    );
    let mut h = harness(app);
    h.run();
    type_personas(&mut h, "2");
    assert!(save_personas_disabled(&h));
    h.ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
    h.get_by_role_and_label(Role::Button, SAVE_PERSONAS).hover();
    h.run();
    h.get_by_label("An agent is still starting; wait until it runs");
    h.get_by_role_and_label(Role::Button, SAVE_PERSONAS).click();
    h.run();
    assert!(fx.calls().is_empty());
}

#[test]
fn personas_save_with_a_pending_request_asks_first() {
    let fx = fx();
    let mut waiting = agent("a1", Status::NeedsYou);
    waiting.blocked_reason = Some(BlockedReason::RepoAccess);
    waiting.repo_request = Some(RepoRequest {
        repo: "portal".into(),
        why: "login".into(),
    });
    let (_log, app) = settings_app(&fx, ctl(&fx, || Ok(Started::StartedByUs(1))), vec![waiting]);
    let mut h = harness(app);
    h.run();
    type_personas(&mut h, "2");
    h.get_by_role_and_label(Role::Button, SAVE_PERSONAS).click();
    h.run();
    h.get_by_label_contains("Saving restarts the daemon, which ends that request");
    h.get_by_role_and_label(Role::Button, "Cancel").click();
    h.run();
    std::thread::sleep(Duration::from_millis(100));
    h.run();
    assert!(fx.calls().is_empty(), "Cancel restarts nothing");
    assert_eq!(
        config::load(&fx.config, &fx.home).unwrap().personas_dir,
        fx.personas(),
        "Cancel writes nothing"
    );

    h.get_by_role_and_label(Role::Button, SAVE_PERSONAS).click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Restart anyway")
        .click();
    assert!(wait_for(&mut h, |a| has_notice(
        a,
        Notice::Success,
        "Daemon restarted"
    )));
    assert_eq!(fx.calls(), vec!["stop", "ensure"]);
    assert_eq!(
        config::load(&fx.config, &fx.home).unwrap().personas_dir,
        fx.home.join("personas2")
    );
}

#[test]
fn personas_note_says_existing_folders_are_not_moved() {
    let fx = fx();
    let (_log, app) = settings_app(&fx, ctl(&fx, || Ok(Started::StartedByUs(1))), vec![]);
    let mut h = harness(app);
    h.run();
    h.get_by_label(PERSONAS_NOTE);
}

#[test]
fn an_empty_personas_folder_is_refused() {
    let fx = fx();
    let (_log, app) = settings_app(&fx, ctl(&fx, || Ok(Started::StartedByUs(1))), vec![]);
    let mut h = harness(app);
    h.run();
    h.get_by_role_and_label(Role::TextInput, "Personas folder")
        .focus();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.key_press(egui::Key::Backspace);
    h.run();
    assert_eq!(
        h.get_by_role_and_label(Role::TextInput, "Personas folder")
            .value()
            .as_deref(),
        Some("")
    );
    h.get_by_role_and_label(Role::Button, SAVE_PERSONAS).click();
    assert!(
        wait_for(&mut h, |a| has_notice(
            a,
            Notice::Error,
            "Enter a folder for the personas"
        )),
        "no error notice"
    );
    std::thread::sleep(Duration::from_millis(100));
    h.run();
    assert!(fx.calls().is_empty(), "nothing restarts");
    assert_eq!(
        config::load(&fx.config, &fx.home).unwrap().personas_dir,
        fx.personas()
    );
    assert!(
        !save_personas_disabled(&h),
        "the failed save ended the restart"
    );
}

/// As for the tasks folder: text typed while the config is being written is the user's next edit.
#[cfg(unix)]
#[test]
fn an_edit_made_while_saving_personas_survives_the_save() {
    let fx = fx();
    let original = std::fs::read_to_string(&fx.config).unwrap();
    std::fs::remove_file(&fx.config).unwrap();
    assert!(std::process::Command::new("mkfifo")
        .arg(&fx.config)
        .status()
        .unwrap()
        .success());
    let (_log, app) = settings_app(&fx, ctl(&fx, || Ok(Started::StartedByUs(1))), vec![]);
    let mut h = harness(app);
    h.run();
    type_personas(&mut h, "2");
    h.get_by_role_and_label(Role::Button, SAVE_PERSONAS).click();
    h.run();
    h.get_by_label("Restarting the daemon…");
    h.get_by_label_contains("Applies to new tasks only");
    type_personas(&mut h, "3");
    std::fs::write(&fx.config, original).unwrap();
    assert!(wait_for(&mut h, |a| has_notice(
        a,
        Notice::Success,
        "Daemon restarted"
    )));
    assert_eq!(
        config::load(&fx.config, &fx.home).unwrap().personas_dir,
        fx.home.join("personas2")
    );
    assert_eq!(
        h.get_by_role_and_label(Role::TextInput, "Personas folder")
            .value()
            .as_deref(),
        Some("~/personas23")
    );
}

/// Replaces the text of the limit field `label`.
fn type_limit(h: &mut Harness<'_, FleetApp>, label: &str, text: &str) {
    let field = h.get_by_role_and_label(Role::TextInput, label);
    field.click();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(text);
    h.run();
}

fn save_limits_disabled(h: &Harness<'_, FleetApp>) -> bool {
    h.get_by_role_and_label(Role::Button, "Save limits")
        .accesskit_node()
        .is_disabled()
}

#[test]
fn settings_save_the_limits() {
    let fx = fx();
    let (_log, app) = settings_app(&fx, ctl(&fx, || Ok(Started::StartedByUs(1))), vec![]);
    let mut h = harness(app.with_limits(Limits::default()));
    h.run();
    for (label, shown) in [
        ("Max breakdown versions", "8"),
        ("Max steps", "20"),
        ("Max parallel specialists", "3"),
    ] {
        assert_eq!(
            h.get_by_role_and_label(Role::TextInput, label)
                .value()
                .as_deref(),
            Some(shown)
        );
    }
    assert!(save_limits_disabled(&h), "nothing changed yet");
    type_limit(&mut h, "Max breakdown versions", "4");
    type_limit(&mut h, "Max steps", "30");
    type_limit(&mut h, "Max parallel specialists", "2");
    h.get_by_role_and_label(Role::Button, "Save limits").click();
    assert!(
        wait_for(&mut h, |a| has_notice(
            a,
            Notice::Success,
            "Daemon restarted"
        )),
        "no Daemon restarted notice"
    );
    assert_eq!(fx.calls(), vec!["stop", "ensure"]);
    let saved = config::load(&fx.config, &fx.home).unwrap();
    assert_eq!(
        saved.limits,
        Limits {
            max_breakdown_versions: 4,
            max_steps: 30,
            max_parallel_specialists: 2,
        }
    );
    assert_eq!(saved.tasks_root, fx.root(), "the folders are kept");
    assert!(save_limits_disabled(&h), "the saved limits are now current");
}

#[test]
fn a_limit_that_is_not_a_positive_number_cannot_be_saved() {
    let fx = fx();
    let (_log, app) = settings_app(&fx, ctl(&fx, || Ok(Started::StartedByUs(1))), vec![]);
    let mut h = harness(app);
    h.run();
    for bad in ["0", "x", ""] {
        type_limit(&mut h, "Max steps", bad);
        assert!(save_limits_disabled(&h), "{bad:?}");
    }
    assert!(fx.calls().is_empty());
}
