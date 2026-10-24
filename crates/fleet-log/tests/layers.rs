use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use fleet_log::{build_with_filter, parse, LogConfig, LogHandle, Notice, Stderr, DEFAULT_FILTER};
use tracing::subscriber::with_default;
use tracing::{debug, info, info_span, warn};
use tracing_subscriber::filter::Targets;

// Always the built-in filter: `build` reads FLEET_LOG, which would make these
// tests depend on the developer's shell.
fn sub(file: Option<PathBuf>) -> (impl tracing::Subscriber + Send + Sync, LogHandle) {
    let filter: Targets = DEFAULT_FILTER.parse().unwrap();
    build_with_filter(
        LogConfig {
            file,
            stderr: Stderr::Never,
        },
        filter,
    )
}

#[test]
fn events_reach_file_and_subscribers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gui.log");
    let (s, handle) = sub(Some(path.clone()));
    let rx = handle.subscribe();
    with_default(s, || {
        info!(target: "fleet_gui::app", agent_id = "7f3a", "repo added");
    });

    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 1);
    let from_file = parse(lines[0]).unwrap();
    assert_eq!(from_file.target, "fleet_gui::app");
    assert_eq!(from_file.message, "repo added");
    assert_eq!(
        from_file.fields,
        vec![("agent_id".to_string(), "7f3a".to_string())]
    );
    assert_eq!(rx.try_recv().unwrap(), from_file);
    assert!(handle.file_error().is_none());
}

#[test]
fn notify_field_is_kept() {
    let (s, handle) = sub(None);
    let rx = handle.subscribe();
    with_default(s, || {
        info!(target: "fleet_gui::app", notify = "success", "saved");
    });
    assert_eq!(rx.try_recv().unwrap().notice(), Some(Notice::Success));
}

#[test]
fn default_filter_caps_third_parties() {
    let (s, handle) = sub(None);
    let rx = handle.subscribe();
    with_default(s, || {
        info!(target: "winit", "dropped");
        warn!(target: "winit", "kept winit");
        info!(target: "fleet_daemon::agents", "kept daemon");
        debug!(target: "fleet_daemon::agents", "dropped debug");
    });
    let got: Vec<String> = rx.try_iter().map(|e| e.message).collect();
    assert_eq!(got, vec!["kept winit", "kept daemon"]);
}

#[test]
fn unwritable_file_keeps_the_fan_out() {
    let dir = tempfile::tempdir().unwrap();
    let ro = dir.path().join("ro");
    std::fs::create_dir(&ro).unwrap();
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o500)).unwrap();

    let (s, handle) = sub(Some(ro.join("missing/sub/gui.log")));
    assert!(handle.file_error().is_some());
    let rx = handle.subscribe();
    with_default(s, || info!(target: "fleet_gui::app", "still here"));
    assert_eq!(rx.try_recv().unwrap().message, "still here");

    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn spans_are_not_written() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gui.log");
    let (s, _handle) = sub(Some(path.clone()));
    with_default(s, || {
        let span = info_span!(target: "fleet_gui::app", "spawn", agent_id = "a");
        let _g = span.enter();
        info!(target: "fleet_gui::app", "inside");
    });
    assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 1);
}
