//! fleet-client's daemon control against the real `fleet-daemon` binary.
use fleet_client::config::{load, ClientConfig};
use fleet_client::daemon_control::{
    connect_or_start, ensure_daemon, find_daemon_binary_in, is_up, stop_daemon, LaunchError,
    Started, StopError,
};
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
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// `tag` names the temporary folder, to tell tests apart when one leaves something behind.
fn fixture(tag: &str) -> Fx {
    let tmp = tempfile::Builder::new()
        .prefix(&format!("fleetlife-{tag}-"))
        .tempdir()
        .unwrap();
    let home = tmp.path().to_path_buf();
    let repo = home.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let p = home.join("config.toml");
    std::fs::write(
        &p,
        format!(
            "listen = \"127.0.0.1:{}\"\nstate_dir = \"{}\"\ntasks_root = \"{}\"\n[[repos]]\nname = \"core\"\npath = \"{}\"\n",
            free_port(),
            home.join("state").display(),
            home.join("tasks").display(),
            repo.display()
        ),
    )
    .unwrap();
    isolate_tmux();
    let cfg = load(&p, &home).unwrap();
    Fx {
        _tmp: tmp,
        home,
        cfg,
    }
}

/// Daemons this binary starts inherit the variable, so their tmux never touches the user's own. Set once, behind a
/// `Once` every test passes through before starting any process: a `set_var` racing another thread's spawn is
/// undefined, and per-test values would overwrite each other anyway.
fn isolate_tmux() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        std::env::set_var(
            "FLEET_TMUX_SOCKET",
            format!("fleetlife-{}", std::process::id()),
        )
    });
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

/// Written by a child `sh`, not this process: exec'ing a file fails with ETXTBSY while any process holds it open for
/// writing, and a test thread forking during our own write would hand that fd to its child.
fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    let status = Command::new("sh")
        .args([
            "-c",
            "printf '#!/bin/sh\\n%s\\n' \"$1\" > \"$2\" && chmod 755 \"$2\"",
            "sh",
            body,
        ])
        .arg(&p)
        .status()
        .unwrap();
    assert!(status.success(), "cannot write {}", p.display());
    p
}

#[test]
fn ensure_daemon_starts_the_real_binary_and_a_second_call_reports_already_running() {
    let fx = fixture("start");
    assert!(!is_up(&fx.cfg.base_url()));
    let started = ensure_daemon(&fx.cfg, Path::new(DAEMON), Duration::from_secs(10));
    assert!(is_up(&fx.cfg.base_url()));
    let pid: u32 = std::fs::read_to_string(fx.cfg.pidfile())
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(pid > 1);
    // The pid the client keeps is the one the daemon writes, so it can tell later whether it was replaced.
    assert!(
        matches!(started, Ok(Started::StartedByUs(p)) if p == pid),
        "{started:?}"
    );
    assert!(matches!(
        ensure_daemon(&fx.cfg, Path::new(DAEMON), Duration::from_secs(10)),
        Ok(Started::AlreadyRunning)
    ));
    stop_daemon(&fx.cfg.state_dir, &fx.cfg.base_url()).unwrap();
}

#[test]
fn ensure_daemon_failure_includes_the_log_tail_and_does_not_wait_out_the_timeout() {
    let fx = fixture("fail");
    let bad = script(
        &fx.home,
        "fleet-daemon",
        "echo 'fleet-daemon: tmux 2.7 is too old' >&2; exit 1",
    );
    let started = Instant::now();
    let err = ensure_daemon(&fx.cfg, &bad, Duration::from_secs(10)).unwrap_err();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "waited for a daemon that had already exited"
    );
    let text = err.to_string();
    assert!(text.contains("tmux 2.7 is too old"), "{text}");
    assert!(text.contains("daemon.log"), "{text}");
}

#[test]
fn the_log_tail_is_the_last_twenty_lines() {
    let fx = fixture("tail");
    let noisy = script(
        &fx.home,
        "fleet-daemon",
        "i=1; while [ $i -le 50 ]; do echo \"line $i\" >&2; i=$((i+1)); done; exit 1",
    );
    let text = ensure_daemon(&fx.cfg, &noisy, Duration::from_secs(10))
        .unwrap_err()
        .to_string();
    assert!(
        text.contains("line 50") && text.contains("line 31"),
        "{text}"
    );
    assert!(
        !text.contains("line 30\n") && !text.contains("line 1\n"),
        "{text}"
    );
}

#[test]
fn a_missing_binary_is_a_clear_error() {
    let fx = fixture("missing");
    let err = ensure_daemon(
        &fx.cfg,
        Path::new("/nonexistent/fleet-daemon"),
        Duration::from_secs(2),
    )
    .unwrap_err();
    assert!(matches!(err, LaunchError::CannotStart { .. }), "{err}");
    assert!(
        err.to_string().contains("/nonexistent/fleet-daemon"),
        "{err}"
    );
}

#[test]
fn connect_or_start_uses_a_running_daemon_without_looking_for_the_binary() {
    let fx = fixture("connect");
    assert!(matches!(
        connect_or_start(&fx.cfg, || None, Duration::from_secs(2)),
        Err(LaunchError::NotFound)
    ));
    ensure_daemon(&fx.cfg, Path::new(DAEMON), Duration::from_secs(10)).unwrap();
    let found = connect_or_start(
        &fx.cfg,
        || panic!("looked for the binary"),
        Duration::from_secs(2),
    );
    assert!(matches!(found, Ok(Started::AlreadyRunning)), "{found:?}");
    stop_daemon(&fx.cfg.state_dir, &fx.cfg.base_url()).unwrap();
}

#[test]
fn losing_a_start_race_to_another_window_reports_already_running() {
    use std::io::{Read, Write};
    let fx = fixture("race");
    // The other window's daemon: not answering yet when we look, serving by the time ours gives up.
    let listener = TcpListener::bind(fx.cfg.listen).unwrap();
    std::thread::spawn(move || {
        for (n, conn) in listener.incoming().enumerate() {
            let Ok(mut c) = conn else { return };
            if n == 0 {
                continue;
            }
            let mut buf = [0u8; 1024];
            let _ = c.read(&mut buf);
            let _ = c.write_all(b"HTTP/1.0 200 OK\r\n\r\n{}");
        }
    });
    // Ours fails to bind the port the other one holds.
    let loser = script(
        &fx.home,
        "fleet-daemon-loser",
        "echo 'address in use' >&2; exit 1",
    );
    let started = ensure_daemon(&fx.cfg, &loser, Duration::from_secs(10));
    assert!(
        matches!(started, Ok(Started::AlreadyRunning)),
        "{started:?}"
    );
}

#[test]
fn stop_daemon_ends_the_process_and_removes_the_pidfile() {
    let fx = fixture("stop");
    ensure_daemon(&fx.cfg, Path::new(DAEMON), Duration::from_secs(10)).unwrap();
    stop_daemon(&fx.cfg.state_dir, &fx.cfg.base_url()).unwrap();
    assert!(
        wait_until(|| !is_up(&fx.cfg.base_url())),
        "daemon still answering after stop"
    );
    assert!(
        wait_until(|| !fx.cfg.pidfile().exists()),
        "pidfile left behind"
    );
}

#[test]
fn stop_daemon_refuses_a_pid_that_is_not_a_fleet_daemon() {
    let fx = fixture("refuse");
    std::fs::create_dir_all(&fx.cfg.state_dir).unwrap();
    let mut sleeper = Command::new("sleep").arg("30").spawn().unwrap();
    for pid in [sleeper.id(), std::process::id()] {
        std::fs::write(fx.cfg.pidfile(), format!("{pid}\n")).unwrap();
        let err = stop_daemon(&fx.cfg.state_dir, &fx.cfg.base_url()).unwrap_err();
        assert!(matches!(err, StopError::NotAFleetDaemon { .. }), "{err}");
    }
    assert!(
        sleeper.try_wait().unwrap().is_none(),
        "an unrelated process was signalled"
    );
    let _ = sleeper.kill();
    let _ = sleeper.wait();
}

#[test]
fn stop_daemon_stops_a_daemon_running_under_its_dist_name() {
    // Linux cuts `comm` to 15 characters, so this reads as "fleet-daemon-x8" in `ps`.
    let fx = fixture("dist-stop");
    std::fs::create_dir_all(&fx.cfg.state_dir).unwrap();
    let link = fx
        .home
        .join(format!("fleet-daemon-{}-linux", std::env::consts::ARCH));
    // A symlink, not a copy, for the same ETXTBSY reason as the zombie test below.
    std::os::unix::fs::symlink(
        Command::new("sh")
            .args(["-c", "command -v sleep"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap(),
        &link,
    )
    .unwrap();
    let mut child = Command::new(&link).arg("30").spawn().unwrap();
    std::fs::write(fx.cfg.pidfile(), format!("{}\n", child.id())).unwrap();
    let res = stop_daemon(&fx.cfg.state_dir, &fx.cfg.base_url());
    if res.is_err() {
        let _ = child.kill();
    }
    res.expect("a dist-named daemon is a fleet-daemon");
    child.wait().unwrap();
    assert!(!fx.cfg.pidfile().exists());
}

#[test]
fn a_stale_pidfile_for_a_dead_pid_is_cleaned_not_signalled() {
    let fx = fixture("stale");
    std::fs::create_dir_all(&fx.cfg.state_dir).unwrap();
    let mut done = Command::new("true").spawn().unwrap();
    done.wait().unwrap();
    std::fs::write(fx.cfg.pidfile(), format!("{}\n", done.id())).unwrap();
    stop_daemon(&fx.cfg.state_dir, &fx.cfg.base_url()).unwrap();
    assert!(!fx.cfg.pidfile().exists(), "stale pidfile kept");
}

#[test]
fn a_zombie_fleet_daemon_counts_as_stopped_not_as_still_running() {
    // A daemon that exited but was never reaped shows in `ps` as "fleet-daemon <defunct>". Treating it as alive made
    // the stop wait out its whole timeout and report failure although the daemon had stopped.
    let fx = fixture("zombie");
    std::fs::create_dir_all(&fx.cfg.state_dir).unwrap();
    // A symlink, not a copy: exec'ing a file just written fails with ETXTBSY when another test thread forks while
    // the write fd is open. The kernel names the process after the path it was exec'd by, so `ps` still says
    // fleet-daemon.
    let link = fx.home.join("fleet-daemon");
    std::os::unix::fs::symlink("/usr/bin/true", &link).unwrap();
    let mut child = Command::new(&link).spawn().unwrap();
    let pid = child.id();
    assert!(
        wait_until(|| String::from_utf8_lossy(
            &Command::new("ps")
                .args(["-p", &pid.to_string(), "-o", "stat="])
                .output()
                .unwrap()
                .stdout
        )
        .trim()
        .starts_with('Z')),
        "the child never became a zombie"
    );
    std::fs::write(fx.cfg.pidfile(), format!("{pid}\n")).unwrap();
    let started = Instant::now();
    stop_daemon(&fx.cfg.state_dir, &fx.cfg.base_url()).expect("a zombie is a stopped daemon");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "waited out the timeout on a zombie"
    );
    assert!(!fx.cfg.pidfile().exists());
    child.wait().unwrap();
}

#[test]
fn no_pidfile_and_a_garbage_pidfile_are_reported_not_acted_on() {
    let fx = fixture("garbage");
    assert!(matches!(
        stop_daemon(&fx.cfg.state_dir, &fx.cfg.base_url()),
        Err(StopError::NoPidfile)
    ));
    std::fs::create_dir_all(&fx.cfg.state_dir).unwrap();
    for bad in ["", "abc", "-1", "0", "1 2"] {
        std::fs::write(fx.cfg.pidfile(), bad).unwrap();
        assert!(
            matches!(
                stop_daemon(&fx.cfg.state_dir, &fx.cfg.base_url()),
                Err(StopError::BadPidfile(_))
            ),
            "{bad:?}"
        );
    }
}

#[test]
fn find_daemon_binary_looks_next_to_the_executable_then_on_path() {
    let fx = fixture("find");
    let dir = fx.home.join("bin");
    std::fs::create_dir_all(&dir).unwrap();
    let fake = script(&dir, "fleet-daemon", "exit 0");
    let path = std::env::join_paths([dir.clone(), PathBuf::from("/usr/bin")]).unwrap();
    assert_eq!(
        find_daemon_binary_in(None, Some(path.clone())).as_deref(),
        Some(fake.as_path())
    );
    // Next to the executable wins over PATH.
    let beside = fx.home.join("app");
    std::fs::create_dir_all(&beside).unwrap();
    let near = script(&beside, "fleet-daemon", "exit 0");
    assert_eq!(
        find_daemon_binary_in(Some(beside.join("fleet-gui")), Some(path)).as_deref(),
        Some(near.as_path())
    );
}

#[cfg(target_os = "linux")]
#[test]
fn finds_the_dist_name_beside_the_app() {
    let fx = fixture("finddist");
    let dir = fx.home.join("dist");
    std::fs::create_dir_all(&dir).unwrap();
    let dist_name = format!("fleet-daemon-{}-linux", std::env::consts::ARCH);
    let arch_named = script(&dir, &dist_name, "exit 0");
    let gui = dir.join("fleet-gui");
    assert_eq!(
        find_daemon_binary_in(Some(gui.clone()), None).as_deref(),
        Some(arch_named.as_path())
    );
    // The plain name wins when both exist.
    let plain = script(&dir, "fleet-daemon", "exit 0");
    assert_eq!(
        find_daemon_binary_in(Some(gui), None).as_deref(),
        Some(plain.as_path())
    );
}
