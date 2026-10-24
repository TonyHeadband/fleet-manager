use fleet_log::{build_with_filter, LogConfig, LogHandle, Stderr, DEFAULT_FILTER};
use tracing::subscriber::DefaultGuard;

/// A thread-local subscriber with a log file in a temp dir. kittest runs the app on the test thread, so events
/// logged from `ui()` reach the handle. Keep the guard alive for the whole test.
pub fn test_log() -> (DefaultGuard, LogHandle) {
    // Leaked on purpose: the file layer holds the file open for as long as the guard lives.
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("temp dir")));
    let cfg = LogConfig { file: Some(dir.path().join("gui.log")), stderr: Stderr::Never };
    let (subscriber, handle) = build_with_filter(cfg, DEFAULT_FILTER.parse().expect("DEFAULT_FILTER is valid"));
    (tracing::subscriber::set_default(subscriber), handle)
}

/// Like `test_log`, but the file cannot be opened, so `file_error()` is set.
pub fn test_log_without_file() -> (DefaultGuard, LogHandle) {
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("temp dir")));
    // A path under a regular file can never be created.
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, "").expect("write blocker");
    let cfg = LogConfig { file: Some(blocker.join("gui.log")), stderr: Stderr::Never };
    let (subscriber, handle) = build_with_filter(cfg, DEFAULT_FILTER.parse().expect("DEFAULT_FILTER is valid"));
    (tracing::subscriber::set_default(subscriber), handle)
}
