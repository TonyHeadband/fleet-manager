//! Starting and stopping the daemon from a client. The daemon outlives the client, so it is started
//! detached, with its output in `<state_dir>/daemon.log`, and stopped only through `<state_dir>/daemon.pid`
//! after checking that the pid really is a `fleet-daemon`.
use crate::config::ClientConfig;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const LOG_TAIL_LINES: usize = 20;
const STOP_WAIT: Duration = Duration::from_secs(5);

/// True when something answers the daemon's API at `base_url`.
pub fn is_up(base_url: &str) -> bool {
    let hostport = base_url.trim_start_matches("http://").trim_end_matches('/');
    let Some(addr) = hostport.to_socket_addrs().ok().and_then(|mut a| a.next()) else { return false };
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(500)) else { return false };
    let _ = s.set_read_timeout(Some(Duration::from_millis(1000)));
    let _ = s.set_write_timeout(Some(Duration::from_millis(500)));
    let req = format!("GET /api/ui-schema HTTP/1.0\r\nHost: {hostport}\r\n\r\n");
    if s.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut head = [0u8; 16];
    s.read(&mut head).is_ok_and(|n| head[..n].starts_with(b"HTTP/1.") && head[..n].windows(3).any(|w| w == b"200"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Started {
    AlreadyRunning,
    StartedByUs,
}

#[derive(Debug)]
pub enum LaunchError {
    CannotStart { bin: PathBuf, error: String },
    /// The daemon exited, or never answered; `detail` carries the last lines of its log.
    Failed { reason: String, log: PathBuf, detail: String },
    Unsupported,
}

impl std::fmt::Display for LaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LaunchError::CannotStart { bin, error } => write!(f, "cannot start {}: {error}", bin.display()),
            LaunchError::Failed { reason, log, detail } => {
                write!(f, "fleet-daemon {reason}\nlog: {}\n--- last lines ---\n{detail}", log.display())
            }
            LaunchError::Unsupported => f.write_str("starting the daemon is not supported on this platform"),
        }
    }
}

impl std::error::Error for LaunchError {}

#[derive(Debug)]
pub enum StopError {
    NoPidfile,
    BadPidfile(String),
    NotAFleetDaemon { pid: u32, name: String },
    Signal(String),
    Unsupported,
}

impl std::fmt::Display for StopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StopError::NoPidfile => f.write_str("no daemon.pid: this daemon was not started by a version that writes one"),
            StopError::BadPidfile(s) => write!(f, "daemon.pid does not hold a process id: {s:?}"),
            StopError::NotAFleetDaemon { pid, name } => {
                write!(f, "pid {pid} in daemon.pid is {name:?}, not a fleet-daemon; not signalling it")
            }
            StopError::Signal(s) => write!(f, "could not stop the daemon: {s}"),
            StopError::Unsupported => f.write_str("stopping the daemon is not supported on this platform"),
        }
    }
}

impl std::error::Error for StopError {}

/// Next to the running executable first (an installed pair), then on `PATH`.
pub fn find_daemon_binary() -> Option<PathBuf> {
    let name = if cfg!(windows) { "fleet-daemon.exe" } else { "fleet-daemon" };
    if let Some(p) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(name))) {
        if p.is_file() {
            return Some(p);
        }
    }
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| p.is_file())
}

#[cfg(unix)]
pub use unix::{ensure_daemon, stop_daemon};
#[cfg(not(unix))]
pub use other::{ensure_daemon, stop_daemon};

#[cfg(not(unix))]
mod other {
    use super::*;

    pub fn ensure_daemon(_: &ClientConfig, _: &Path, _: Duration) -> Result<Started, LaunchError> {
        Err(LaunchError::Unsupported)
    }

    pub fn stop_daemon(_: &Path) -> Result<(), StopError> {
        Err(StopError::Unsupported)
    }
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    /// The last lines of the log written after `offset` (so an older run's output is not mistaken for this one).
    fn log_tail(log: &Path, offset: u64) -> String {
        let bytes = std::fs::read(log).unwrap_or_default();
        let new = &bytes[(offset as usize).min(bytes.len())..];
        let text = String::from_utf8_lossy(new);
        let lines: Vec<&str> = text.lines().collect();
        lines[lines.len().saturating_sub(LOG_TAIL_LINES)..].join("\n")
    }

    pub fn ensure_daemon(cfg: &ClientConfig, daemon_bin: &Path, wait: Duration) -> Result<Started, LaunchError> {
        let base = cfg.base_url();
        if is_up(&base) {
            return Ok(Started::AlreadyRunning);
        }
        let cannot = |e: std::io::Error| LaunchError::CannotStart { bin: daemon_bin.to_path_buf(), error: e.to_string() };
        std::fs::create_dir_all(&cfg.state_dir).map_err(cannot)?;
        let log = cfg.log_file();
        let offset = std::fs::metadata(&log).map(|m| m.len()).unwrap_or(0);
        let out = std::fs::OpenOptions::new().create(true).append(true).open(&log).map_err(cannot)?;
        let err = out.try_clone().map_err(cannot)?;
        let mut child = Command::new(daemon_bin)
            .arg("--config")
            .arg(&cfg.path)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err)
            // Its own process group: a Ctrl-C or a closing terminal aimed at the client must not reach it.
            .process_group(0)
            .spawn()
            .map_err(cannot)?;

        let deadline = Instant::now() + wait;
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                return Err(LaunchError::Failed { reason: format!("exited at startup ({status})"), log, detail: log_tail(&cfg.log_file(), offset) });
            }
            if is_up(&base) {
                // Reap it when it eventually exits so it never lingers as a zombie of the client.
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Ok(Started::StartedByUs);
            }
            if Instant::now() >= deadline {
                // It may still come up late; reap it whenever it exits instead of leaving a zombie of the client.
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Err(LaunchError::Failed {
                    reason: format!("did not answer within {}s", wait.as_secs()),
                    log,
                    detail: log_tail(&cfg.log_file(), offset),
                });
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// The command name of `pid`, or `None` when no such process exists. `ps` works on Linux and macOS alike.
    fn process_name(pid: u32) -> Option<String> {
        let out = Command::new("ps").args(["-p", &pid.to_string(), "-o", "comm="]).output().ok()?;
        let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
        // An exited process nobody has reaped still lists as "<name> <defunct>": it is gone for our purposes.
        (out.status.success() && !name.is_empty() && !name.ends_with("<defunct>")).then_some(name)
    }

    fn basename(s: &str) -> &str {
        s.rsplit('/').next().unwrap_or(s)
    }

    pub fn stop_daemon(state_dir: &Path) -> Result<(), StopError> {
        let pidfile = state_dir.join("daemon.pid");
        let text = match std::fs::read_to_string(&pidfile) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(StopError::NoPidfile),
            Err(e) => return Err(StopError::BadPidfile(e.to_string())),
        };
        let pid = match text.trim().parse::<u32>() {
            Ok(p) if p > 1 => p,
            _ => return Err(StopError::BadPidfile(text.trim().to_string())),
        };
        match process_name(pid) {
            None => {
                // The daemon is gone and left its pidfile: nothing to stop.
                let _ = std::fs::remove_file(&pidfile);
                Ok(())
            }
            Some(name) if basename(&name) == "fleet-daemon" => {
                // SAFETY: plain kill(2) on a pid we just identified; no memory is involved.
                if unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) } != 0 {
                    return Err(StopError::Signal(std::io::Error::last_os_error().to_string()));
                }
                let deadline = Instant::now() + STOP_WAIT;
                while process_name(pid).is_some() {
                    if Instant::now() >= deadline {
                        return Err(StopError::Signal(format!("pid {pid} still running after {}s", STOP_WAIT.as_secs())));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                let _ = std::fs::remove_file(&pidfile);
                Ok(())
            }
            Some(name) => Err(StopError::NotAFleetDaemon { pid, name }),
        }
    }
}
