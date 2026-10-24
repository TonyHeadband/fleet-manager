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
            "listen = \"{listen}\"\nstate_dir = \"{}\"\n[[repos]]\nname = \"core\"\npath = \"{}\"\n",
            dir.join("state").display(),
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
fn a_missing_config_names_the_path_it_looked_for() {
    let out = command(Path::new("/nonexistent/fleet.toml"), "fleetmain-none").output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("/nonexistent/fleet.toml"));
}
