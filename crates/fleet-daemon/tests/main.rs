use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
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
    c.arg("--config").arg(cfg).env("FLEET_TMUX_SOCKET", socket).stdout(Stdio::piped()).stderr(Stdio::piped());
    c
}

fn get(port: u16, path: &str) -> (u16, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(s, "GET {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n").unwrap();
    let mut raw = String::new();
    s.read_to_string(&mut raw).unwrap();
    (raw.split_whitespace().nth(1).unwrap().parse().unwrap(), raw.split_once("\r\n\r\n").unwrap().1.to_string())
}

#[test]
fn the_binary_serves_the_api_on_the_address_it_reports() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path(), "127.0.0.1:0");
    let mut child = command(&cfg, &format!("fleetmain-{}", std::process::id())).spawn().unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let _d = Daemon(child);
    let port: u16 = line.trim().rsplit(':').next().unwrap().parse().unwrap_or_else(|_| panic!("unexpected first line: {line:?}"));
    assert!(line.contains("listening on http://127.0.0.1:"), "{line}");

    let (status, body) = get(port, "/api/agents");
    assert_eq!(status, 200);
    assert!(body.contains("\"agents\":[]"), "{body}");
    assert!(dir.path().join("state/fleet.db").is_file(), "database not created in state_dir");
}

#[test]
fn a_non_loopback_listen_address_exits_with_an_error_and_never_binds() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path(), "0.0.0.0:0");
    let out = command(&cfg, &format!("fleetmain-bad-{}", std::process::id())).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not loopback"), "{}", String::from_utf8_lossy(&out.stderr));
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
    let mut child = command(&cfg, &format!("fleetmain-pid-{}", std::process::id())).spawn().unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let pidfile = dir.path().join("state/daemon.pid");
    let text = std::fs::read_to_string(&pidfile).expect("pidfile written before the daemon says it is listening");
    assert_eq!(text.trim(), child.id().to_string());
    assert!(text.ends_with('\n'));

    let status = Command::new("kill").arg("-TERM").arg(child.id().to_string()).status().unwrap();
    assert!(status.success());
    let exit = child.wait().unwrap();
    assert!(exit.success(), "SIGTERM must be a graceful exit, got {exit:?}");
    assert!(!pidfile.exists(), "pidfile left behind after a graceful exit");
}

#[test]
fn the_binary_prints_its_version_without_starting() {
    let out = Command::new(env!("CARGO_BIN_EXE_fleet-daemon")).arg("--version").output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), format!("fleet-daemon {}", env!("CARGO_PKG_VERSION")));
}

#[test]
fn sigterm_stops_the_daemon_promptly_even_with_an_open_event_stream() {
    // A connected client (the app, a browser tab) holds an SSE stream open; graceful shutdown must not wait for it.
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path(), "127.0.0.1:0");
    let mut child = command(&cfg, &format!("fleetmain-sse-{}", std::process::id())).spawn().unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let port: u16 = line.trim().rsplit(':').next().unwrap().parse().unwrap();

    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(stream, "GET /api/events HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n").unwrap();
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
    let mut seen = String::new();
    let mut buf = [0u8; 512];
    while !seen.contains("snapshot") {
        let n = stream.read(&mut buf).unwrap();
        assert!(n > 0, "stream closed before the snapshot");
        seen.push_str(&String::from_utf8_lossy(&buf[..n]));
    }

    assert!(Command::new("kill").arg("-TERM").arg(child.id().to_string()).status().unwrap().success());
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
    assert!(!dir.path().join("state/daemon.pid").exists(), "pidfile left behind");
    drop(stream);
}
