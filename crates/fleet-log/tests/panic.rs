use fleet_log::{build_with_filter, install_panic_hook, parse, LogConfig, Stderr, DEFAULT_FILTER};
use tracing::subscriber::with_default;

#[test]
fn a_panic_is_written_to_the_log_with_its_location() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gui.log");
    let (s, _handle) = build_with_filter(
        LogConfig {
            file: Some(path.clone()),
            stderr: Stderr::Never,
        },
        DEFAULT_FILTER.parse().unwrap(),
    );
    install_panic_hook();
    with_default(s, || {
        let _ = std::panic::catch_unwind(|| panic!("boom {}", 7));
    });
    let _ = std::panic::take_hook();

    let text = std::fs::read_to_string(&path).unwrap();
    let entry = parse(text.lines().next().unwrap()).unwrap();
    assert!(
        entry.message.starts_with("panic: boom 7 at ") && entry.message.contains("panic.rs:"),
        "{}",
        entry.message
    );
}
