//! Starting and stopping the daemon on every OS, without the real daemon (the daemon's own tests cover that on Linux).
use fleet_client::config::load;
use fleet_client::daemon_control::{ensure_daemon, is_fleet_daemon, LaunchError};
use std::path::Path;
use std::time::Duration;

fn config(dir: &Path) -> fleet_client::config::ClientConfig {
    let p = dir.join("config.toml");
    std::fs::write(
        &p,
        format!(
            "listen = \"127.0.0.1:0\"\nstate_dir = {:?}\ntasks_root = {:?}\n",
            dir.join("state").display().to_string(),
            dir.join("tasks").display().to_string(),
        ),
    )
    .unwrap();
    load(&p, dir).unwrap()
}

#[test]
fn names_with_an_exe_suffix_are_fleet_daemons() {
    assert!(is_fleet_daemon("fleet-daemon.exe"));
    assert!(is_fleet_daemon("fleet-daemon-x86_64-linux"));
    assert!(!is_fleet_daemon("fleet-gui.exe"));
}

/// A stand-in that exits at once: proves the detached start works here, where it used to be `Unsupported`.
#[test]
fn a_binary_that_exits_at_once_is_reported_with_its_log() {
    let t = tempfile::tempdir().unwrap();
    let cfg = config(t.path());
    let quitter = if cfg!(windows) {
        "C:\\Windows\\System32\\whoami.exe"
    } else {
        // /usr/bin, not /bin: macOS has no /bin/true.
        "/usr/bin/true"
    };
    let err = ensure_daemon(&cfg, Path::new(quitter), Duration::from_secs(5)).unwrap_err();
    assert!(matches!(err, LaunchError::Failed { .. }), "{err}");
}

/// Sleeps long enough to be stopped; run only as a stand-in daemon by the test below.
#[test]
#[ignore]
fn stand_in_daemon_sleeps() {
    std::thread::sleep(Duration::from_secs(30));
}

// Unix falls back to SIGTERM; covered by the daemon's gui_lifecycle tests.
#[cfg(windows)]
#[test]
fn stop_without_an_endpoint_is_an_error_on_windows() {
    use fleet_client::daemon_control::{stop_daemon, StopError};
    let t = tempfile::tempdir().unwrap();
    let cfg = config(t.path());
    std::fs::create_dir_all(&cfg.state_dir).unwrap();
    // A live process that passes the name check but serves no endpoint: this test binary copied as fleet-daemon.exe,
    // running the ignored sleeper above.
    let stand_in = t.path().join("fleet-daemon.exe");
    std::fs::copy(std::env::current_exe().unwrap(), &stand_in).unwrap();
    let mut child = std::process::Command::new(&stand_in)
        .args([
            "--exact",
            "stand_in_daemon_sleeps",
            "--ignored",
            "--nocapture",
        ])
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    std::fs::write(
        cfg.state_dir.join("daemon.pid"),
        format!("{}\n", child.id()),
    )
    .unwrap();
    let res = stop_daemon(&cfg.state_dir, &cfg.base_url());
    let _ = child.kill();
    let _ = child.wait();
    assert!(matches!(res, Err(StopError::Signal(_))), "{res:?}");
}

/// The built daemon beside this test binary (`<target>/<profile>/deps/daemon_start-*.exe`), which Cargo builds for
/// the workspace run but not for `-p fleet-client` alone; the test says so when it is missing, and fails under CI.
#[cfg(windows)]
fn built_daemon() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bin = exe.parent()?.parent()?.join("fleet-daemon.exe");
    bin.is_file().then_some(bin)
}

#[cfg(windows)]
#[test]
fn ensure_daemon_starts_the_real_daemon_and_stop_daemon_stops_it() {
    use fleet_client::daemon_control::{stop_daemon, Started};
    let Some(bin) = built_daemon() else {
        // CI runs the workspace, which builds the daemon: a missing one there is a broken setup, not a skip.
        assert!(
            std::env::var_os("CI").is_none(),
            "fleet-daemon.exe is not built beside the tests"
        );
        eprintln!(
            "SKIPPED: fleet-daemon.exe is not built; run `cargo build -p fleet-daemon` first"
        );
        return;
    };
    let t = tempfile::tempdir().unwrap();
    let cfg = config(t.path());
    let started = ensure_daemon(&cfg, &bin, Duration::from_secs(30)).unwrap();
    let stopped = stop_daemon(&cfg.state_dir, &cfg.base_url());
    assert!(matches!(started, Started::StartedByUs(_)), "{started:?}");
    stopped.unwrap();
}
