//! fleet-client's daemon control against the real `fleet-daemon` binary.
use fleet_client::config::{load, ClientConfig};
use fleet_client::daemon_control::{ensure_daemon, find_daemon_binary, is_up, stop_daemon, LaunchError, StopError, Started};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const DAEMON: &str = env!("CARGO_BIN_EXE_fleet-daemon");

struct Fx {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    cfg: ClientConfig,
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn fixture(tag: &str) -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_path_buf();
    let repo = home.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let p = home.join("config.toml");
    std::fs::write(
        &p,
        format!(
            "listen = \"127.0.0.1:{}\"\nstate_dir = \"{}\"\nprojects_root = \"{}\"\n[[repos]]\nname = \"core\"\npath = \"{}\"\n",
            free_port(),
            home.join("state").display(),
            home.join("projects").display(),
            repo.display()
        ),
    )
    .unwrap();
    // The daemon inherits this, so test servers never touch the user's own tmux.
    std::env::set_var("FLEET_TMUX_SOCKET", format!("fleetlife-{tag}-{}", std::process::id()));
    let cfg = load(&p, &home).unwrap();
    Fx { _tmp: tmp, home, cfg }
}

fn wait_until(mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let p = dir.join(name);
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

#[test]
fn ensure_daemon_starts_the_real_binary_and_a_second_call_reports_already_running() {
    let fx = fixture("start");
    assert!(!is_up(&fx.cfg.base_url()));
    assert!(matches!(ensure_daemon(&fx.cfg, Path::new(DAEMON), Duration::from_secs(10)), Ok(Started::StartedByUs)));
    assert!(is_up(&fx.cfg.base_url()));
    let pid: u32 = std::fs::read_to_string(fx.cfg.pidfile()).unwrap().trim().parse().unwrap();
    assert!(pid > 1);
    assert!(matches!(ensure_daemon(&fx.cfg, Path::new(DAEMON), Duration::from_secs(10)), Ok(Started::AlreadyRunning)));
    stop_daemon(&fx.cfg.state_dir).unwrap();
}

#[test]
fn ensure_daemon_failure_includes_the_log_tail_and_does_not_wait_out_the_timeout() {
    let fx = fixture("fail");
    let bad = script(&fx.home, "fleet-daemon", "echo 'fleet-daemon: tmux 2.7 is too old' >&2; exit 1");
    let started = Instant::now();
    let err = ensure_daemon(&fx.cfg, &bad, Duration::from_secs(10)).unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(5), "waited for a daemon that had already exited");
    let text = err.to_string();
    assert!(text.contains("tmux 2.7 is too old"), "{text}");
    assert!(text.contains("daemon.log"), "{text}");
}

#[test]
fn the_log_tail_is_the_last_twenty_lines() {
    let fx = fixture("tail");
    let noisy = script(&fx.home, "fleet-daemon", "i=1; while [ $i -le 50 ]; do echo \"line $i\" >&2; i=$((i+1)); done; exit 1");
    let text = ensure_daemon(&fx.cfg, &noisy, Duration::from_secs(10)).unwrap_err().to_string();
    assert!(text.contains("line 50") && text.contains("line 31"), "{text}");
    assert!(!text.contains("line 30\n") && !text.contains("line 1\n"), "{text}");
}

#[test]
fn a_missing_binary_is_a_clear_error() {
    let fx = fixture("missing");
    let err = ensure_daemon(&fx.cfg, Path::new("/nonexistent/fleet-daemon"), Duration::from_secs(2)).unwrap_err();
    assert!(matches!(err, LaunchError::CannotStart { .. }), "{err}");
    assert!(err.to_string().contains("/nonexistent/fleet-daemon"), "{err}");
}

#[test]
fn stop_daemon_ends_the_process_and_removes_the_pidfile() {
    let fx = fixture("stop");
    ensure_daemon(&fx.cfg, Path::new(DAEMON), Duration::from_secs(10)).unwrap();
    stop_daemon(&fx.cfg.state_dir).unwrap();
    assert!(wait_until(|| !is_up(&fx.cfg.base_url())), "daemon still answering after stop");
    assert!(wait_until(|| !fx.cfg.pidfile().exists()), "pidfile left behind");
}

#[test]
fn stop_daemon_refuses_a_pid_that_is_not_a_fleet_daemon() {
    let fx = fixture("refuse");
    std::fs::create_dir_all(&fx.cfg.state_dir).unwrap();
    let mut sleeper = Command::new("sleep").arg("30").spawn().unwrap();
    for pid in [sleeper.id(), std::process::id()] {
        std::fs::write(fx.cfg.pidfile(), format!("{pid}\n")).unwrap();
        let err = stop_daemon(&fx.cfg.state_dir).unwrap_err();
        assert!(matches!(err, StopError::NotAFleetDaemon { .. }), "{err}");
    }
    assert!(sleeper.try_wait().unwrap().is_none(), "an unrelated process was signalled");
    let _ = sleeper.kill();
    let _ = sleeper.wait();
}

#[test]
fn a_stale_pidfile_for_a_dead_pid_is_cleaned_not_signalled() {
    let fx = fixture("stale");
    std::fs::create_dir_all(&fx.cfg.state_dir).unwrap();
    let mut done = Command::new("true").spawn().unwrap();
    done.wait().unwrap();
    std::fs::write(fx.cfg.pidfile(), format!("{}\n", done.id())).unwrap();
    stop_daemon(&fx.cfg.state_dir).unwrap();
    assert!(!fx.cfg.pidfile().exists(), "stale pidfile kept");
}

#[test]
fn a_zombie_fleet_daemon_counts_as_stopped_not_as_still_running() {
    // A daemon that exited but was never reaped shows in `ps` as "fleet-daemon <defunct>". Treating it as alive made
    // the stop wait out its whole timeout and report failure although the daemon had stopped.
    let fx = fixture("zombie");
    std::fs::create_dir_all(&fx.cfg.state_dir).unwrap();
    let copy = fx.home.join("fleet-daemon");
    std::fs::copy("/usr/bin/true", &copy).unwrap();
    let mut child = Command::new(&copy).spawn().unwrap();
    let pid = child.id();
    assert!(
        wait_until(|| String::from_utf8_lossy(&Command::new("ps").args(["-p", &pid.to_string(), "-o", "stat="]).output().unwrap().stdout).trim().starts_with('Z')),
        "the child never became a zombie"
    );
    std::fs::write(fx.cfg.pidfile(), format!("{pid}\n")).unwrap();
    let started = Instant::now();
    stop_daemon(&fx.cfg.state_dir).expect("a zombie is a stopped daemon");
    assert!(started.elapsed() < Duration::from_secs(2), "waited out the timeout on a zombie");
    assert!(!fx.cfg.pidfile().exists());
    child.wait().unwrap();
}

#[test]
fn no_pidfile_and_a_garbage_pidfile_are_reported_not_acted_on() {
    let fx = fixture("garbage");
    assert!(matches!(stop_daemon(&fx.cfg.state_dir), Err(StopError::NoPidfile)));
    std::fs::create_dir_all(&fx.cfg.state_dir).unwrap();
    for bad in ["", "abc", "-1", "0", "1 2"] {
        std::fs::write(fx.cfg.pidfile(), bad).unwrap();
        assert!(matches!(stop_daemon(&fx.cfg.state_dir), Err(StopError::BadPidfile(_))), "{bad:?}");
    }
}

#[test]
fn find_daemon_binary_looks_next_to_the_executable_then_on_path() {
    let fx = fixture("find");
    let dir = fx.home.join("bin");
    std::fs::create_dir_all(&dir).unwrap();
    let fake = script(&dir, "fleet-daemon", "exit 0");
    let old = std::env::var_os("PATH").unwrap_or_default();
    std::env::set_var("PATH", format!("{}:{}", dir.display(), old.to_string_lossy()));
    assert_eq!(find_daemon_binary().as_deref(), Some(fake.as_path()));
    std::env::set_var("PATH", old);
}
