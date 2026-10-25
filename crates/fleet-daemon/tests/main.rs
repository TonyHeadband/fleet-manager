use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};

/// A daemon a test started, stopped with its tmux server however the test ends: a failed assertion must not leave
/// either running (their open binaries also keep a worktree's build folder from being deleted on NFS).
struct Daemon {
    child: Child,
    socket: String,
}

impl Daemon {
    fn spawn(cfg: &Path, socket: &str) -> Daemon {
        Daemon::new(command(cfg, socket).spawn().unwrap(), socket)
    }

    fn new(child: Child, socket: &str) -> Daemon {
        Daemon {
            child,
            socket: socket.to_string(),
        }
    }
}

impl std::ops::Deref for Daemon {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.child
    }
}

impl std::ops::DerefMut for Daemon {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.child
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = Command::new("tmux")
            .args(["-L", &self.socket, "kill-server"])
            .status();
    }
}

fn config(dir: &Path, listen: &str) -> std::path::PathBuf {
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let p = dir.join("config.toml");
    std::fs::write(
        &p,
        format!(
            "listen = \"{listen}\"\nstate_dir = \"{}\"\nprojects_root = \"{}\"\n[[repos]]\nname = \"core\"\npath = \"{}\"\n",
            dir.join("state").display(),
            dir.join("projects").display(),
            repo.display()
        ),
    )
    .unwrap();
    p
}

fn command(cfg: &Path, socket: &str) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_fleet-daemon"));
    c.arg("--config")
        .arg(cfg)
        .env("FLEET_TMUX_SOCKET", socket)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    c
}

fn get(port: u16, path: &str) -> (u16, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(s, "GET {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n").unwrap();
    let mut raw = String::new();
    s.read_to_string(&mut raw).unwrap();
    (
        raw.split_whitespace().nth(1).unwrap().parse().unwrap(),
        raw.split_once("\r\n\r\n").unwrap().1.to_string(),
    )
}

#[test]
fn the_binary_serves_the_api_on_the_address_it_reports() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path(), "127.0.0.1:0");
    let mut child = Daemon::spawn(&cfg, &format!("fleetmain-{}", std::process::id()));
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let port: u16 = line
        .trim()
        .rsplit(':')
        .next()
        .unwrap()
        .parse()
        .unwrap_or_else(|_| panic!("unexpected first line: {line:?}"));
    assert!(line.contains("listening on http://127.0.0.1:"), "{line}");

    let (status, body) = get(port, "/api/agents");
    assert_eq!(status, 200);
    assert!(body.contains("\"agents\":[]"), "{body}");
    assert!(
        dir.path().join("state/fleet.db").is_file(),
        "database not created in state_dir"
    );
}

#[test]
fn a_non_loopback_listen_address_exits_with_an_error_and_never_binds() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path(), "0.0.0.0:0");
    let out = command(&cfg, &format!("fleetmain-bad-{}", std::process::id()))
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("not loopback"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn an_unreadable_config_names_the_path_it_looked_for() {
    // A missing file now means defaults, so use a directory: it exists but cannot be read as a file.
    let dir = tempfile::tempdir().unwrap();
    let out = command(dir.path(), "fleetmain-none").output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains(&dir.path().display().to_string()));
}

#[test]
fn pidfile_holds_the_daemons_pid_while_running_and_is_gone_after_sigterm() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path(), "127.0.0.1:0");
    let mut child = Daemon::spawn(&cfg, &format!("fleetmain-pid-{}", std::process::id()));
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let pidfile = dir.path().join("state/daemon.pid");
    let text = std::fs::read_to_string(&pidfile)
        .expect("pidfile written before the daemon says it is listening");
    assert_eq!(text.trim(), child.id().to_string());
    assert!(text.ends_with('\n'));

    let status = Command::new("kill")
        .arg("-TERM")
        .arg(child.id().to_string())
        .status()
        .unwrap();
    assert!(status.success());
    let exit = child.wait().unwrap();
    assert!(
        exit.success(),
        "SIGTERM must be a graceful exit, got {exit:?}"
    );
    assert!(
        !pidfile.exists(),
        "pidfile left behind after a graceful exit"
    );
}

#[test]
fn the_binary_prints_its_version_without_starting() {
    let out = Command::new(env!("CARGO_BIN_EXE_fleet-daemon"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!("fleet-daemon {}", env!("CARGO_PKG_VERSION"))
    );
}

/// Runs the binary with `args` and a fresh HOME, so a run that wrongly starts leaves files where the test sees them.
/// A run that starts never exits on its own, so it is killed after a few seconds instead of hanging the test.
fn run_args(args: &[&str]) -> (std::process::Output, tempfile::TempDir) {
    let home = tempfile::tempdir().unwrap();
    let socket = format!("fleetmain-args-{}", std::process::id());
    let mut c = Command::new(env!("CARGO_BIN_EXE_fleet-daemon"));
    c.args(args)
        .env("HOME", home.path())
        .env("FLEET_TMUX_SOCKET", &socket)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = Daemon::new(c.spawn().unwrap(), &socket);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while child.try_wait().unwrap().is_none() {
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let mut out = std::process::Output {
        status: child.wait().unwrap(),
        stdout: Vec::new(),
        stderr: Vec::new(),
    };
    child
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut out.stdout)
        .unwrap();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_end(&mut out.stderr)
        .unwrap();
    (out, home)
}

fn is_empty(dir: &Path) -> bool {
    std::fs::read_dir(dir).unwrap().next().is_none()
}

#[test]
fn help_prints_the_usage_without_starting() {
    for flag in ["--help", "-h"] {
        let (out, home) = run_args(&[flag]);
        assert!(out.status.success(), "{flag}: {out:?}");
        let text = String::from_utf8_lossy(&out.stdout);
        for word in [
            "Usage: fleet-daemon",
            "--config",
            "--version",
            "--help",
            "SIGTERM",
        ] {
            assert!(text.contains(word), "{flag}: missing {word:?} in:\n{text}");
        }
        assert!(is_empty(home.path()), "{flag} started the daemon");
    }
}

#[test]
fn an_unknown_argument_exits_with_usage_and_never_starts() {
    let (out, home) = run_args(&["--confg", "x.toml"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("--confg") && err.contains("--help"), "{err}");
    assert!(out.stdout.is_empty());
    assert!(
        is_empty(home.path()),
        "an unknown argument started the daemon"
    );
}

#[test]
fn config_without_a_path_exits_with_usage_and_never_starts() {
    let (out, home) = run_args(&["--config"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("--config needs a path"));
    assert!(
        is_empty(home.path()),
        "a bare --config started the daemon on the default config"
    );
}

#[test]
fn sigterm_stops_the_daemon_promptly_even_with_an_open_event_stream() {
    // A connected client (the app, a browser tab) holds an SSE stream open; graceful shutdown must not wait for it.
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path(), "127.0.0.1:0");
    let mut child = Daemon::spawn(&cfg, &format!("fleetmain-sse-{}", std::process::id()));
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let port: u16 = line.trim().rsplit(':').next().unwrap().parse().unwrap();

    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        stream,
        "GET /api/events HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n"
    )
    .unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let mut seen = String::new();
    let mut buf = [0u8; 512];
    while !seen.contains("snapshot") {
        let n = stream.read(&mut buf).unwrap();
        assert!(n > 0, "stream closed before the snapshot");
        seen.push_str(&String::from_utf8_lossy(&buf[..n]));
    }

    assert!(Command::new("kill")
        .arg("-TERM")
        .arg(child.id().to_string())
        .status()
        .unwrap()
        .success());
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        if started.elapsed() > std::time::Duration::from_secs(8) {
            let _ = child.kill();
            panic!("the daemon was still running 8 s after SIGTERM with an event stream open");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    assert!(status.success(), "{status:?}");
    assert!(
        !dir.path().join("state/daemon.pid").exists(),
        "pidfile left behind"
    );
    drop(stream);
}

#[test]
fn daemon_writes_parsed_lines_to_daemon_log() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path(), "127.0.0.1:0");
    // Started the way the app does: stdout and stderr appended to daemon.log itself. A stderr layer that wrote
    // when not on a terminal would then put every line into the file twice.
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let log_path = state.join("daemon.log");
    let sink = || {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .unwrap()
    };
    let socket = format!("fleetmain-log-{}", std::process::id());
    let mut c = command(&cfg, &socket);
    c.stdout(sink()).stderr(sink());
    let _d = Daemon::new(c.spawn().unwrap(), &socket);

    let started = std::time::Instant::now();
    let text = loop {
        let text = std::fs::read_to_string(&log_path).unwrap_or_default();
        if text.contains("listening on http://") {
            break text;
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "daemon never listened: {text:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let entries = fleet_log::parse_file(&text);
    assert!(
        entries.iter().any(
            |e| e.level == Some(fleet_log::Level::Info) && e.target.starts_with("fleet_daemon")
        ),
        "no parsed info line from fleet_daemon in {text:?}"
    );
    let lines: Vec<&str> = text
        .lines()
        .filter(|l| fleet_log::parse(l).is_some())
        .collect();
    let mut unique = lines.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(
        lines.len(),
        unique.len(),
        "a line was written twice: {text}"
    );
    let started_entries = entries
        .iter()
        .filter(|e| e.message == "daemon started")
        .count();
    assert_eq!(started_entries, 1, "{text}");
}

fn send(port: u16, method: &str, path: &str, body: &str) -> (u16, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        s,
        "{method} {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut raw = String::new();
    s.read_to_string(&mut raw).unwrap();
    (
        raw.split_whitespace().nth(1).unwrap().parse().unwrap(),
        raw.split_once("\r\n\r\n").unwrap().1.to_string(),
    )
}

fn field<'a>(e: &'a fleet_log::Entry, name: &str) -> Option<&'a str> {
    e.fields
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

/// Daemon → Log is only useful if daemon.log says what happened to agents, not just that the daemon started.
#[test]
fn daemon_log_records_the_agent_lifecycle_and_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path(), "127.0.0.1:0");
    let claude = format!("{}/tests/bin/claude", env!("CARGO_MANIFEST_DIR"));
    let mut text = std::fs::read_to_string(&cfg).unwrap();
    text = format!("claude_command = [\"{claude}\"]\n{text}");
    std::fs::write(&cfg, text).unwrap();
    let socket = format!("fleetmain-life-{}", std::process::id());
    let mut child = Daemon::spawn(&cfg, &socket);
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let port: u16 = line
        .trim()
        .rsplit(':')
        .next()
        .unwrap()
        .parse()
        .unwrap_or_else(|_| panic!("unexpected first line: {line:?}"));

    let (status, body) = send(
        port,
        "POST",
        "/api/agents",
        r#"{"actor":"test","prompt":"log me","task_slug":"log-me"}"#,
    );
    assert_eq!(status, 200, "{body}");
    let spawned: serde_json::Value = serde_json::from_str(&body).unwrap();
    let id = spawned["agent"]["id"].as_str().unwrap().to_string();
    let (status, body) = send(port, "POST", &format!("/api/agents/{id}/stop"), "");
    assert_eq!(status, 204, "{body}");

    assert!(Command::new("kill")
        .arg("-TERM")
        .arg(child.id().to_string())
        .status()
        .unwrap()
        .success());
    let exit = child.wait().unwrap();
    assert!(exit.success(), "{exit:?}");

    let text = std::fs::read_to_string(dir.path().join("state/daemon.log")).unwrap();
    let entries = fleet_log::parse_file(&text);
    let find = |msg: &str| {
        entries
            .iter()
            .find(|e| e.message == msg)
            .unwrap_or_else(|| panic!("no {msg:?} line in {text}"))
    };
    let spawned = find("agent spawned");
    assert_eq!(field(spawned, "agent_id"), Some(id.as_str()), "{text}");
    assert_eq!(field(spawned, "project"), Some("log-me"), "{text}");
    assert_eq!(field(spawned, "repos"), Some("0"), "{text}");
    let stopped = find("agent stopped");
    assert_eq!(field(stopped, "agent_id"), Some(id.as_str()), "{text}");
    find("shutdown signal received");
    find("daemon stopped");
}
