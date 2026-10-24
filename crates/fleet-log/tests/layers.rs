use std::path::PathBuf;

use fleet_log::{build_with_filter, parse, LogConfig, LogHandle, Notice, Stderr, DEFAULT_FILTER};
use tracing::subscriber::with_default;
use tracing::{debug, error, info, info_span, warn};
use tracing_subscriber::filter::Targets;

// Always the built-in filter: `build` reads FLEET_LOG, which would make these
// tests depend on the developer's shell.
fn sub(file: Option<PathBuf>) -> (impl tracing::Subscriber + Send + Sync, LogHandle) {
    sub_with(file, DEFAULT_FILTER)
}

fn sub_with(file: Option<PathBuf>, spec: &str) -> (impl tracing::Subscriber + Send + Sync, LogHandle) {
    let filter: Targets = spec.parse().unwrap();
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
    // A path below a regular file cannot be created by any user, root included.
    let blocker = tempfile::NamedTempFile::new().unwrap();
    let (s, handle) = sub(Some(blocker.path().join("sub/gui.log")));
    assert!(handle.file_error().is_some());
    let rx = handle.subscribe();
    with_default(s, || info!(target: "fleet_gui::app", "still here"));
    assert_eq!(rx.try_recv().unwrap().message, "still here");
}

#[test]
fn log_crate_records_keep_target_filter_and_clean_fields() {
    let (s, handle) = sub(None);
    let rx = handle.subscribe();
    let tracer = tracing_log::LogTracer::new();
    let record = |level: log::Level| {
        log::Record::builder()
            .args(format_args!("x"))
            .level(level)
            .target("winit")
            .module_path(Some("winit::window"))
            .file(Some("window.rs"))
            .line(Some(7))
            .build()
    };
    with_default(s, || {
        log::Log::log(&tracer, &record(log::Level::Info));
        log::Log::log(&tracer, &record(log::Level::Warn));
    });
    let got: Vec<_> = rx.try_iter().collect();
    assert_eq!(got.len(), 1, "info from winit is dropped, warn kept");
    assert_eq!(got[0].target, "winit");
    assert_eq!(got[0].level, Some(fleet_log::Level::Warn));
    assert_eq!(got[0].message, "x");
    assert!(got[0].fields.is_empty(), "{:?}", got[0].fields);
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

/// FLEET_LOG is a debugging knob for the files; it must not switch off what the
/// footer shows.
#[test]
fn a_narrow_filter_keeps_footer_notices() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gui.log");
    let (s, handle) = sub_with(Some(path.clone()), "fleet_daemon=debug");
    let rx = handle.subscribe();
    with_default(s, || {
        error!(target: "fleet_gui::app", notify = "error", "failed");
        info!(target: "fleet_gui::app", notify = "success", "saved");
        debug!(target: "fleet_gui::app", "too chatty");
        info!(target: "winit", "third party");
        debug!(target: "fleet_daemon::agents", "daemon detail");
    });
    let got: Vec<String> = rx.try_iter().map(|e| e.message).collect();
    assert_eq!(got, vec!["failed", "saved", "daemon detail"]);
    let text = std::fs::read_to_string(&path).unwrap();
    let in_file: Vec<String> = text.lines().map(|l| parse(l).unwrap().message).collect();
    assert_eq!(in_file, vec!["daemon detail"], "the file still honours the filter");
}

#[test]
fn a_warn_filter_keeps_info_notices() {
    let (s, handle) = sub_with(None, "warn");
    let rx = handle.subscribe();
    with_default(s, || {
        info!(target: "fleet_gui::app", notify = "success", "saved");
        info!(target: "winit", "third party");
    });
    let got: Vec<String> = rx.try_iter().map(|e| e.message).collect();
    assert_eq!(got, vec!["saved"]);
}

/// A writer the test can read back, standing in for stderr.
#[derive(Clone, Default)]
struct Captured(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn stderr_output(stderr: Stderr, is_terminal: bool) -> String {
    let out = Captured::default();
    let sink = fleet_log::StderrSink { writer: Box::new(out.clone()), is_terminal };
    let (s, _handle) = fleet_log::build_with_sink(LogConfig { file: None, stderr }, DEFAULT_FILTER.parse().unwrap(), sink);
    with_default(s, || {
        info!(target: "fleet_gui", n = 1, "to the terminal");
        debug!(target: "fleet_gui", "below the filter");
    });
    let bytes = out.0.lock().unwrap().clone();
    String::from_utf8(bytes).unwrap()
}

#[test]
fn stderr_gets_the_filtered_lines_only_on_a_terminal() {
    let text = stderr_output(Stderr::IfTerminal, true);
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines.len(), 1, "{text}");
    let e = parse(lines[0]).expect("one rendered line");
    assert_eq!((e.target.as_str(), e.message.as_str()), ("fleet_gui", "to the terminal"));
    assert_eq!(stderr_output(Stderr::IfTerminal, false), "", "not a terminal: nothing");
    assert_eq!(stderr_output(Stderr::Never, true), "", "Never: nothing, even on a terminal");
}
