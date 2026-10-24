mod common;

use eframe::egui::{self, accesskit::Role, Vec2};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use fleet_client::http::FleetClient;
use fleet_gui::app::FleetApp;
use fleet_gui::shell::DaemonCtl;
use fleet_gui::views::settings::{tail, SettingsTab, TAIL};
use fleet_gui::views::View;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Fx {
    tmp: tempfile::TempDir,
}

impl Fx {
    fn state(&self) -> PathBuf {
        self.tmp.path().join("state")
    }

    fn write(&self, name: &str, text: &str) {
        std::fs::write(self.state().join(name), text).unwrap();
    }
}

/// A state dir holding whichever of `gui.log` and `daemon.log` are given.
fn fx(app: Option<&str>, daemon: Option<&str>) -> Fx {
    let fx = Fx { tmp: tempfile::tempdir().unwrap() };
    std::fs::create_dir_all(fx.state()).unwrap();
    if let Some(t) = app {
        fx.write("gui.log", t);
    }
    if let Some(t) = daemon {
        fx.write("daemon.log", t);
    }
    fx
}

fn ctl(state_dir: PathBuf) -> DaemonCtl {
    DaemonCtl {
        ensure: Arc::new(|| Ok(fleet_client::daemon_control::Started::AlreadyRunning)),
        stop: Arc::new(|| Ok(())),
        config_path: state_dir.join("config.toml"),
        state_dir,
        listen: "127.0.0.1:7878".into(),
        we_started: false,
    }
}

fn rows(h: &Harness<'_, FleetApp>) -> Vec<String> {
    h.query_all_by_role(Role::Label).filter_map(|n| n.accesskit_node().value().or_else(|| n.accesskit_node().label())).filter(|t| t.contains(" · ")).collect()
}

fn wait_for(h: &mut Harness<'_, FleetApp>, mut done: impl FnMut(&Harness<'_, FleetApp>) -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        h.run();
        if done(h) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// A detached app on Settings with its live log reaching it, not yet on the Log tab. Keep the guard alive.
fn app_on_settings(fx: &Fx) -> (tracing::subscriber::DefaultGuard, Harness<'static, FleetApp>) {
    let (guard, handle) = common::log::test_log();
    let mut app = FleetApp::detached(FleetClient::new("http://127.0.0.1:1"), egui::Context::default()).with_daemon(ctl(fx.state())).with_log(handle);
    app.set_view(View::Settings);
    let mut h = Harness::builder().with_max_steps(60).with_size(Vec2::new(1400.0, 800.0)).build_ui_state(|ui, app: &mut FleetApp| app.ui(ui), app);
    h.run();
    (guard, h)
}

fn show_log(h: &mut Harness<'_, FleetApp>) {
    h.get_by_role_and_label(Role::Button, "Log").click();
    h.run();
    assert_eq!(h.state().settings_tab(), SettingsTab::Log);
}

fn open(fx: &Fx) -> (tracing::subscriber::DefaultGuard, Harness<'static, FleetApp>) {
    let (guard, mut h) = app_on_settings(fx);
    show_log(&mut h);
    (guard, h)
}

/// Clicks the last button called `label`: the source chip `Daemon` comes after the tab of the same name.
fn press(h: &mut Harness<'_, FleetApp>, label: &str) {
    let all: Vec<_> = h.query_all_by_role_and_label(Role::Button, label).collect();
    all.last().unwrap_or_else(|| panic!("no {label} button")).click();
    h.run();
}

const APP: &str = "2026-10-24T10:00:02Z INFO  fleet_gui app-line\n";
const DAEMON: &str = "2026-10-24T10:00:01Z INFO  fleet_daemon first-daemon\n2026-10-24T10:00:03Z INFO  fleet_daemon last-daemon\n";

#[test]
fn both_merges_by_time() {
    let fx = fx(Some(APP), Some(DAEMON));
    let (_g, mut h) = open(&fx);
    assert!(wait_for(&mut h, |h| rows(h).len() == 3), "rows: {:?}", rows(&h));
    let r = rows(&h);
    assert!(r[0].contains("first-daemon") && r[1].contains("app-line") && r[2].contains("last-daemon"), "{r:?}");
    assert!(r[0].starts_with("10:00:01 · fleet_daemon "), "{r:?}");

    press(&mut h, "App");
    assert!(rows(&h).iter().all(|t| !t.contains("daemon")));
    press(&mut h, "Daemon");
    let r = rows(&h);
    assert_eq!(r.len(), 2, "{r:?}");
    assert!(r.iter().all(|t| t.contains("-daemon")));
}

#[test]
fn ties_list_the_app_line_first() {
    let fx = fx(Some("2026-10-24T10:00:01Z INFO  fleet_gui app-line\n"), Some("2026-10-24T10:00:01Z INFO  fleet_daemon daemon-line\n"));
    let (_g, mut h) = open(&fx);
    assert!(wait_for(&mut h, |h| rows(h).len() == 2));
    let r = rows(&h);
    assert!(r[0].contains("app-line") && r[1].contains("daemon-line"), "{r:?}");
}

#[test]
fn search_matches_fields() {
    let fx = fx(Some("2026-10-24T10:00:01Z INFO  fleet_gui spawned\tagent_id=7f3a\n2026-10-24T10:00:02Z INFO  fleet_gui other\tagent_id=0000\n"), None);
    let (_g, mut h) = open(&fx);
    assert!(wait_for(&mut h, |h| rows(h).len() == 2));
    let field = h.get_by_role(Role::TextInput);
    field.click();
    field.type_text("7F3A");
    h.run();
    let r = rows(&h);
    assert_eq!(r.len(), 1, "{r:?}");
    assert!(r[0].contains("agent_id=7f3a"), "{r:?}");
}

#[test]
fn debug_is_hidden_by_default() {
    let fx = fx(Some("2026-10-24T10:00:01Z DEBUG fleet_gui noisy\n2026-10-24T10:00:02Z INFO  fleet_gui quiet\n"), None);
    let (_g, mut h) = open(&fx);
    assert!(wait_for(&mut h, |h| !rows(h).is_empty()));
    let r = rows(&h);
    assert_eq!(r.len(), 1, "{r:?}");
    assert!(r[0].contains("quiet"));
    press(&mut h, "debug");
    assert_eq!(rows(&h).len(), 2);
}

#[test]
fn level_toggle_hides_errors() {
    let fx = fx(Some("2026-10-24T10:00:01Z ERROR fleet_gui broken\n2026-10-24T10:00:02Z WARN  fleet_gui careful\n2026-10-24T10:00:03Z INFO  fleet_gui fine\n"), None);
    let (_g, mut h) = open(&fx);
    assert!(wait_for(&mut h, |h| rows(h).len() == 3));
    press(&mut h, "error");
    let r = rows(&h);
    assert_eq!(r.len(), 2, "{r:?}");
    assert!(r.iter().all(|t| !t.contains("broken")));
    press(&mut h, "warning");
    assert_eq!(rows(&h).len(), 1);
    press(&mut h, "info");
    assert!(rows(&h).is_empty());
}

#[test]
fn unparsed_lines_ignore_level_toggles() {
    let fx = fx(None, Some("2026-10-24T10:00:01Z INFO  fleet_daemon started\nthread 'main' panicked at src/main.rs:1\n"));
    let (_g, mut h) = open(&fx);
    press(&mut h, "Daemon");
    assert!(wait_for(&mut h, |h| rows(h).len() == 2), "rows: {:?}", rows(&h));
    for level in ["error", "warning", "info"] {
        press(&mut h, level);
    }
    let r = rows(&h);
    assert_eq!(r.len(), 1, "{r:?}");
    assert!(r[0].contains("panicked at src/main.rs:1"));
    assert!(r[0].starts_with("10:00:01 · "), "an unparsed line takes the previous line's time: {r:?}");
}

#[test]
fn missing_daemon_log_says_so() {
    let fx = fx(Some(APP), None);
    let (_g, mut h) = open(&fx);
    let path = fx.state().join("daemon.log").display().to_string();
    assert!(wait_for(&mut h, |h| h.query_by_label(&format!("No daemon log at {path}")).is_some()));
    h.get_by_label_contains("app-line");
    press(&mut h, "App");
    assert!(h.query_by_label_contains("No daemon log at").is_none(), "the App source does not need it");
    press(&mut h, "Daemon");
    h.get_by_label(&format!("No daemon log at {path}"));
}

#[test]
fn missing_app_log_says_so() {
    let fx = fx(None, Some(DAEMON));
    let (_g, mut h) = open(&fx);
    let path = fx.state().join("gui.log").display().to_string();
    assert!(wait_for(&mut h, |h| h.query_by_label(&format!("No app log at {path}")).is_some()));
}

#[test]
fn tail_reads_the_end_of_a_big_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.log");
    let mut text = String::new();
    let mut n = 0;
    while text.len() < 3 * 1024 * 1024 {
        text.push_str(&format!("2026-10-24T10:00:01Z INFO  fleet_gui line {n:08}\n"));
        n += 1;
    }
    std::fs::write(&path, &text).unwrap();
    let (got, end) = tail(&path, TAIL).unwrap();
    assert!(got.len() as u64 <= TAIL);
    assert!(got.len() as u64 > TAIL - 100, "reads nearly all it may");
    assert!(got.starts_with("2026-10-24T10:00:01Z"), "starts at a line start: {:?}", &got[..40]);
    assert!(got.ends_with(&format!("line {:08}\n", n - 1)));
    assert_eq!(end, text.len() as u64);
    assert!(text.ends_with(&got));

    // A small file comes back whole.
    let small = dir.path().join("small.log");
    std::fs::write(&small, "a\nb\n").unwrap();
    assert_eq!(tail(&small, TAIL).unwrap(), ("a\nb\n".to_string(), 4));
    // A cut that lands exactly on a line start keeps that line.
    assert_eq!(tail(&small, 2).unwrap(), ("b\n".to_string(), 4));
    assert_eq!(tail(&dir.path().join("nope.log"), TAIL).unwrap_err().kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn new_app_entries_appear_live() {
    let fx = fx(Some(APP), None);
    let (_g, mut h) = open(&fx);
    assert!(wait_for(&mut h, |h| rows(h).iter().any(|t| t.contains("app-line"))));
    tracing::warn!("live-event");
    h.run();
    let r = rows(&h);
    assert!(r.last().unwrap().contains("live-event"), "{r:?}");
}

#[test]
fn an_app_line_in_the_file_and_the_live_log_is_listed_once() {
    let fx = fx(None, None);
    let (_g, mut h) = app_on_settings(&fx);
    tracing::warn!("dup-event");
    h.run();
    // What the app's own file layer holds by now, after an older line from a previous run.
    let live = h.state().log_entries().iter().find(|e| e.message == "dup-event").map(fleet_log::render).unwrap();
    fx.write("gui.log", &format!("2020-01-01T00:00:00Z INFO  fleet_gui old-run\n{live}\n"));
    show_log(&mut h);
    assert!(wait_for(&mut h, |h| rows(h).iter().any(|t| t.contains("old-run"))), "rows: {:?}", rows(&h));
    let r = rows(&h);
    assert_eq!(r.iter().filter(|t| t.contains("dup-event")).count(), 1, "{r:?}");
    assert!(r[0].contains("old-run"), "{r:?}");
}

#[test]
fn the_daemon_file_is_followed_and_survives_truncation() {
    let fx = fx(None, Some(DAEMON));
    let (_g, mut h) = open(&fx);
    press(&mut h, "Daemon");
    assert!(wait_for(&mut h, |h| rows(h).len() == 2));
    append(&fx.state().join("daemon.log"), "2026-10-24T10:00:04Z INFO  fleet_daemon appended\n");
    assert!(wait_for(&mut h, |h| rows(h).len() == 3), "rows: {:?}", rows(&h));
    assert!(rows(&h)[2].contains("appended"));

    fx.write("daemon.log", "2026-10-24T11:00:00Z INFO  fleet_daemon rotated\n");
    assert!(wait_for(&mut h, |h| rows(h).len() == 1), "rows: {:?}", rows(&h));
    assert!(rows(&h)[0].contains("rotated"));
}

#[test]
fn a_partial_last_line_waits_for_its_newline() {
    let fx = fx(None, Some("2026-10-24T10:00:01Z INFO  fleet_daemon whole\n2026-10-24T10:00:02Z INFO  fleet_da"));
    let (_g, mut h) = open(&fx);
    press(&mut h, "Daemon");
    assert!(wait_for(&mut h, |h| !rows(h).is_empty()));
    assert_eq!(rows(&h).len(), 1);
    append(&fx.state().join("daemon.log"), "emon finished\n");
    assert!(wait_for(&mut h, |h| rows(h).len() == 2), "rows: {:?}", rows(&h));
    assert!(rows(&h)[1].contains("finished"));
}

fn append(path: &Path, text: &str) {
    use std::io::Write;
    std::fs::OpenOptions::new().append(true).open(path).unwrap().write_all(text.as_bytes()).unwrap();
}
