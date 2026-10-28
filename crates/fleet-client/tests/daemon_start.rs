//! Starting and stopping the daemon on every OS, without the real daemon (the daemon's own tests cover that on Linux).
use fleet_client::config::load;
use fleet_client::daemon_control::{ensure_daemon, is_fleet_daemon, LaunchError};
use std::path::Path;
use std::time::Duration;

fn config(dir: &Path) -> fleet_client::config::ClientConfig {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let p = dir.join("config.toml");
    std::fs::write(
        &p,
        format!(
            "listen = \"127.0.0.1:{port}\"\nstate_dir = {:?}\ntasks_root = {:?}\n",
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
        "/bin/true"
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
