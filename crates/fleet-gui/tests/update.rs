mod common;

use eframe::egui::{self, accesskit::Role, Vec2};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use fleet_client::config::{GuiSettings, SettingsWriter};
use fleet_client::daemon_control::Started;
use fleet_client::update::Release;
use fleet_gui::app::AppResult;
use fleet_gui::shell::{ReleasesFn, Root, ToolsFn};
use fleet_gui::views::View;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const CURRENT: &str = env!("CARGO_PKG_VERSION");
const NOTICE: &str = "is available (you have";

struct Fx {
    tmp: tempfile::TempDir,
    settings: PathBuf,
    config: PathBuf,
    /// How many times the releases were asked for.
    asked: Arc<AtomicUsize>,
}

fn fx() -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let settings = tmp.path().join("gui.toml");
    // Nothing answers on this address: the daemon's answers are handed in.
    let config = tmp.path().join("config.toml");
    std::fs::write(&config, "listen = \"127.0.0.1:1\"\n").unwrap();
    Fx {
        tmp,
        settings,
        config,
        asked: Arc::new(AtomicUsize::new(0)),
    }
}

fn release(version: &str) -> Release {
    Release {
        version: version.into(),
        notes: "Faster boards.".into(),
        page: format!("https://example.invalid/releases/v{version}"),
    }
}

fn releases(fx: &Fx, answer: Result<Release, String>) -> ReleasesFn {
    let asked = fx.asked.clone();
    Arc::new(move || {
        asked.fetch_add(1, Ordering::SeqCst);
        answer.clone()
    })
}

fn all_found() -> ToolsFn {
    use fleet_api::ToolId::*;
    let list = fleet_api::ToolList {
        tools: [Git, GitBash, Claude, Gh]
            .map(|tool| fleet_api::ToolStatus {
                tool,
                required: true,
                version: Some("1.0".into()),
                error: None,
            })
            .to_vec(),
        checked_at: 1,
    };
    Arc::new(move || Ok(list.clone()))
}

fn root(fx: &Fx, answer: Result<Release, String>) -> Root {
    // Started by this window: no daemon version offer in the way.
    root_with(fx, answer, Started::StartedByUs(1))
}

fn root_with(fx: &Fx, answer: Result<Release, String>, started: Started) -> Root {
    let (_guard, log) = common::log::test_log();
    Root::new(
        egui::Context::default(),
        "http://127.0.0.1:1".into(),
        fx.settings.clone(),
        PathBuf::from("/state"),
        fx.config.clone(),
        fx.tmp.path().to_path_buf(),
        Arc::new(move || Ok(started)),
        Arc::new(|| Ok(())),
        log,
    )
    .with_tools(all_found())
    .with_recheck(all_found())
    .with_releases(releases(fx, answer))
}

fn harness(r: Root) -> Harness<'static, Root> {
    Harness::builder()
        .with_max_steps(60)
        .with_size(Vec2::new(1000.0, 700.0))
        .with_context_setup(fleet_gui::theme::apply)
        .build_ui_state(|ui, r: &mut Root| r.ui(ui), r)
}

fn wait_for(h: &mut Harness<'static, Root>, done: impl Fn(&Root) -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while !done(h.state()) && Instant::now() < end {
        h.step();
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_label(h: &mut Harness<'static, Root>, label: &str) {
    let end = Instant::now() + Duration::from_secs(5);
    while h.query_by_label_contains(label).is_none() && Instant::now() < end {
        h.step();
        std::thread::sleep(Duration::from_millis(20));
    }
    h.get_by_label_contains(label);
}

fn settle(h: &mut Harness<'static, Root>) {
    for _ in 0..10 {
        h.step();
        std::thread::sleep(Duration::from_millis(20));
    }
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

/// The board's daemon answers, with `personas` as the persona list.
fn hand_in(h: &mut Harness<'static, Root>, personas: Vec<fleet_api::Persona>) {
    let app = h.state_mut().app_mut().unwrap();
    app.handle_result(AppResult::Schema(Ok(fleet_api::default_ui_schema())));
    app.handle_result(AppResult::Repos(Ok(vec![])));
    app.handle_result(AppResult::Personas(Ok(fleet_api::PersonaList {
        personas,
        claude_command: vec!["claude".into()],
        gh_command: "gh".into(),
        tasks_root: "/tasks".into(),
    })));
}

/// The board, past startup, with a ready persona: nothing else to show.
fn board(fx: &Fx, answer: Result<Release, String>) -> Harness<'static, Root> {
    let mut h = harness(root(fx, answer));
    wait_for(&mut h, |r| r.is_running());
    hand_in(&mut h, vec![ready_persona()]);
    h
}

fn on_disk(fx: &Fx) -> GuiSettings {
    SettingsWriter::shared().flush();
    GuiSettings::load(&fx.settings)
}

fn open_settings(h: &mut Harness<'static, Root>) {
    h.state_mut().app_mut().unwrap().set_view(View::Settings);
    h.step();
    h.step();
}

#[test]
fn a_newer_release_shows_the_modal() {
    let fx = fx();
    let mut h = board(&fx, Ok(release("99.0.0")));
    wait_label(
        &mut h,
        &format!("fleet-manager v99.0.0 is available (you have v{CURRENT})"),
    );
    h.get_by_label("Faster boards.");
    h.get_by_role_and_label(Role::Button, "Download");
    h.get_by_label("Skip this version");
    h.get_by_role_and_label(Role::Button, "Later").click();
    settle(&mut h);
    assert!(h.query_by_label_contains(NOTICE).is_none());
    assert_eq!(on_disk(&fx).skipped_version, None, "Later skips nothing");
    open_settings(&mut h);
    h.get_by_label("v99.0.0 available");
    h.get_by_role_and_label(Role::Link, "Download");
}

#[test]
fn skip_this_version_hides_it_next_time() {
    let fx = fx();
    let mut h = board(&fx, Ok(release("99.0.0")));
    wait_label(&mut h, NOTICE);
    h.get_by_label("Skip this version").click();
    h.step();
    h.get_by_role_and_label(Role::Button, "Later").click();
    settle(&mut h);
    assert!(h.query_by_label_contains(NOTICE).is_none());
    assert_eq!(on_disk(&fx).skipped_version.as_deref(), Some("99.0.0"));

    // The next start finds the same release: no notice, but Settings still says it is out.
    let asked = fx.asked.load(Ordering::SeqCst);
    let mut h = board(&fx, Ok(release("99.0.0")));
    let end = Instant::now() + Duration::from_secs(5);
    while fx.asked.load(Ordering::SeqCst) == asked && Instant::now() < end {
        h.step();
        std::thread::sleep(Duration::from_millis(20));
    }
    settle(&mut h);
    assert!(h.query_by_label_contains(NOTICE).is_none());
    open_settings(&mut h);
    h.get_by_label("v99.0.0 available");

    // A later release shows again.
    let mut h = board(&fx, Ok(release("99.1.0")));
    wait_label(&mut h, "fleet-manager v99.1.0 is available");
}

#[test]
fn a_failed_check_only_changes_the_settings_line() {
    let fx = fx();
    let mut h = board(&fx, Err("GitHub answered HTTP 403".into()));
    let end = Instant::now() + Duration::from_secs(5);
    while fx.asked.load(Ordering::SeqCst) == 0 && Instant::now() < end {
        h.step();
        std::thread::sleep(Duration::from_millis(20));
    }
    settle(&mut h);
    assert!(h.query_by_label_contains(NOTICE).is_none());
    assert!(
        h.query_by_label_contains("403").is_none(),
        "no footer alert"
    );
    open_settings(&mut h);
    h.get_by_label("Could not check for updates");
}

#[test]
fn an_answered_check_says_up_to_date_in_settings() {
    let fx = fx();
    let mut h = board(&fx, Ok(release(CURRENT)));
    open_settings(&mut h);
    wait_label(&mut h, &format!("Version v{CURRENT} — up to date"));
    assert!(h.query_by_label_contains(NOTICE).is_none());
}

#[test]
fn no_notice_during_first_run() {
    let fx = fx();
    std::fs::remove_file(&fx.config).unwrap();
    let mut h = harness(root(&fx, Ok(release("99.0.0"))));
    settle(&mut h);
    assert!(h.state().is_first_run());
    h.get_by_label("Next →").click();
    wait_for(&mut h, |r| r.is_requirements());
    settle(&mut h);
    assert_eq!(
        fx.asked.load(Ordering::SeqCst),
        0,
        "no check while the wizard's first steps show"
    );
    h.get_by_label("Continue").click();
    wait_for(&mut h, |r| r.is_running());
    // No persona yet: the wizard goes on with the persona steps, over the board.
    hand_in(&mut h, vec![]);
    wait_label(&mut h, "Your first persona");
    settle(&mut h);
    assert!(h.query_by_label_contains(NOTICE).is_none());
}

#[test]
fn the_notice_waits_for_another_dialog() {
    let fx = fx();
    // A daemon this window did not start, of another version: the restart offer wants the screen too.
    let r = root_with(&fx, Ok(release("99.0.0")), Started::AlreadyRunning).with_version(Arc::new(
        || {
            Ok(Some(fleet_api::VersionInfo {
                version: "0.0.1".into(),
            }))
        },
    ));
    let mut h = harness(r);
    wait_for(&mut h, |r| r.is_running());
    hand_in(&mut h, vec![ready_persona()]);
    h.state_mut()
        .app_mut()
        .unwrap()
        .handle_stream(fleet_client::stream::StreamMsg::Snapshot(
            fleet_api::Snapshot {
                seq: 1,
                agents: vec![],
            },
        ));
    wait_label(&mut h, "The background service is from v0.0.1");
    let end = Instant::now() + Duration::from_secs(5);
    while fx.asked.load(Ordering::SeqCst) == 0 && Instant::now() < end {
        h.step();
        std::thread::sleep(Duration::from_millis(20));
    }
    settle(&mut h);
    assert!(
        h.query_by_label_contains(NOTICE).is_none(),
        "one dialog at a time"
    );
    h.get_by_role_and_label(Role::Button, "Keep using it")
        .click();
    wait_label(&mut h, "fleet-manager v99.0.0 is available");
    assert!(h
        .query_by_label_contains("The background service is from")
        .is_none());
}
