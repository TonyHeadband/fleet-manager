//! Starting and stopping the daemon from a client. The daemon outlives the client, so it is started
//! detached, with its output in `<state_dir>/daemon.log`, and stopped only through `<state_dir>/daemon.pid`
//! after checking that the pid really is a `fleet-daemon`.
use crate::config::ClientConfig;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const LOG_TAIL_LINES: usize = 20;
const STOP_WAIT: Duration = Duration::from_secs(5);

/// True when something answers the daemon's API at `base_url`.
pub fn is_up(base_url: &str) -> bool {
    let hostport = base_url.trim_start_matches("http://").trim_end_matches('/');
    let Some(addr) = hostport.to_socket_addrs().ok().and_then(|mut a| a.next()) else {
        return false;
    };
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(500)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_millis(1000)));
    let _ = s.set_write_timeout(Some(Duration::from_millis(500)));
    let req = format!("GET /api/ui-schema HTTP/1.0\r\nHost: {hostport}\r\n\r\n");
    if s.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut head = [0u8; 16];
    s.read(&mut head).is_ok_and(|n| {
        head[..n].starts_with(b"HTTP/1.") && head[..n].windows(3).any(|w| w == b"200")
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Started {
    AlreadyRunning,
    /// The pid of the daemon this client started: the `fleet-daemon` process itself, which writes the same pid to
    /// `daemon.pid`.
    StartedByUs(u32),
}

#[derive(Debug)]
pub enum LaunchError {
    CannotStart {
        bin: PathBuf,
        error: String,
    },
    /// The daemon exited, or never answered; `detail` carries the last lines of its log.
    Failed {
        reason: String,
        log: PathBuf,
        detail: String,
    },
    /// No daemon answers and there is no binary to start one with.
    NotFound,
}

impl std::fmt::Display for LaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LaunchError::CannotStart { bin, error } => {
                write!(f, "cannot start {}: {error}", bin.display())
            }
            LaunchError::Failed {
                reason,
                log,
                detail,
            } => {
                write!(
                    f,
                    "fleet-daemon {reason}\nlog: {}\n--- last lines ---\n{detail}",
                    log.display()
                )
            }
            LaunchError::NotFound => {
                f.write_str("fleet-daemon was not found next to fleet-gui or on PATH")
            }
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
}

impl std::fmt::Display for StopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StopError::NoPidfile => f.write_str(
                "no daemon.pid: this daemon was not started by a version that writes one",
            ),
            StopError::BadPidfile(s) => write!(f, "daemon.pid does not hold a process id: {s:?}"),
            StopError::NotAFleetDaemon { pid, name } => {
                write!(
                    f,
                    "pid {pid} in daemon.pid is {name:?}, not a fleet-daemon; not signalling it"
                )
            }
            StopError::Signal(s) => write!(f, "could not stop the daemon: {s}"),
        }
    }
}

impl std::error::Error for StopError {}

/// Whether `daemon.pid` in `state_dir` still names `pid`: the daemon a client started may since have been restarted
/// by someone else, and is then no longer that client's to stop.
pub fn pidfile_names(state_dir: &Path, pid: u32) -> bool {
    std::fs::read_to_string(state_dir.join("daemon.pid"))
        .is_ok_and(|t| t.trim().parse::<u32>() == Ok(pid))
}

/// Next to the running executable first (an installed pair, including the `fleet-daemon-<arch>-linux` name
/// `build.sh` gives it in `dist/`), then on `PATH`.
pub fn find_daemon_binary() -> Option<PathBuf> {
    find_daemon_binary_in(std::env::current_exe().ok(), std::env::var_os("PATH"))
}

/// `find_daemon_binary` for a given executable and `PATH`, so tests never have to change the process environment.
#[doc(hidden)]
pub fn find_daemon_binary_in(
    exe: Option<PathBuf>,
    path: Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "fleet-daemon.exe"
    } else {
        "fleet-daemon"
    };
    if let Some(dir) = exe.as_deref().and_then(Path::parent) {
        if let Some(p) = std::iter::once(name.to_string())
            // `build.sh` names the binaries in `dist/` after the architecture; only looked for beside the app.
            .chain(
                cfg!(target_os = "linux")
                    .then(|| format!("fleet-daemon-{}-linux", std::env::consts::ARCH)),
            )
            .map(|n| dir.join(n))
            .find(|p| p.is_file())
        {
            return Some(p);
        }
    }
    std::env::split_paths(&path?)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// Uses a daemon that already answers as is, and looks for the binary (with `find`, normally
/// `find_daemon_binary`) only when one has to be started: a GUI run from elsewhere can still connect.
pub fn connect_or_start(
    cfg: &ClientConfig,
    find: impl FnOnce() -> Option<PathBuf>,
    wait: Duration,
) -> Result<Started, LaunchError> {
    if is_up(&cfg.base_url()) {
        return Ok(Started::AlreadyRunning);
    }
    let bin = find().ok_or(LaunchError::NotFound)?;
    ensure_daemon(cfg, &bin, wait)
}

/// The last lines of the log written after `offset` (so an older run's output is not mistaken for this one).
fn log_tail(log: &Path, offset: u64) -> String {
    let bytes = std::fs::read(log).unwrap_or_default();
    let new = &bytes[(offset as usize).min(bytes.len())..];
    let text = String::from_utf8_lossy(new);
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(LOG_TAIL_LINES)..].join("\n")
}

pub fn ensure_daemon(
    cfg: &ClientConfig,
    daemon_bin: &Path,
    wait: Duration,
) -> Result<Started, LaunchError> {
    let base = cfg.base_url();
    if is_up(&base) {
        return Ok(Started::AlreadyRunning);
    }
    let cannot = |e: std::io::Error| LaunchError::CannotStart {
        bin: daemon_bin.to_path_buf(),
        error: e.to_string(),
    };
    std::fs::create_dir_all(&cfg.state_dir).map_err(cannot)?;
    let log = cfg.log_file();
    let offset = std::fs::metadata(&log).map(|m| m.len()).unwrap_or(0);
    let out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .map_err(cannot)?;
    let err = out.try_clone().map_err(cannot)?;
    let mut cmd = Command::new(daemon_bin);
    cmd.arg("--config")
        .arg(&cfg.path)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err);
    // Its own process group (Unix) or console group (Windows): a Ctrl-C or a closing terminal aimed at the client
    // must not reach it.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    let mut child = cmd.spawn().map_err(cannot)?;

    let deadline = Instant::now() + wait;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            // Two windows starting at once: the one that lost the port exits with "address in use" while the
            // other's daemon serves. That daemon is what we wanted, just not ours.
            if is_up(&base) {
                return Ok(Started::AlreadyRunning);
            }
            return Err(LaunchError::Failed {
                reason: format!("exited at startup ({status})"),
                log,
                detail: log_tail(&cfg.log_file(), offset),
            });
        }
        if is_up(&base) {
            // Reap it when it eventually exits so it never lingers as a zombie of the client.
            let pid = child.id();
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            // The daemon writes daemon.pid before it serves, so one that answers without naming our child is
            // another window's; ours then fails on the taken port.
            return Ok(if pidfile_names(&cfg.state_dir, pid) {
                Started::StartedByUs(pid)
            } else {
                Started::AlreadyRunning
            });
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

/// The process name of `pid`, or `None` when no such process exists (or it exited and awaits reaping).
fn process_name(pid: u32) -> Option<String> {
    use sysinfo::{Pid, ProcessStatus, ProcessesToUpdate, System};
    let pid = Pid::from_u32(pid);
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    let p = sys.process(pid)?;
    (p.status() != ProcessStatus::Zombie).then(|| p.name().to_string_lossy().into_owned())
}

/// The plain name, or a dist name (`fleet-daemon-<arch>-linux`), with `.exe` on Windows. Linux cuts process names to
/// 15 characters, so a dist name can read as `fleet-daemon-x8`: any `fleet-daemon-` prefix counts.
pub fn is_fleet_daemon(name: &str) -> bool {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let base = base.strip_suffix(".exe").unwrap_or(base);
    base == "fleet-daemon" || base.starts_with("fleet-daemon-")
}

pub fn stop_daemon(state_dir: &Path, base_url: &str) -> Result<(), StopError> {
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
        Some(name) if is_fleet_daemon(&name) => {
            if !ask_to_stop(base_url) {
                signal_stop(pid)?;
            }
            let deadline = Instant::now() + STOP_WAIT;
            while process_name(pid).is_some() {
                if Instant::now() >= deadline {
                    return Err(StopError::Signal(format!(
                        "pid {pid} still running after {}s",
                        STOP_WAIT.as_secs()
                    )));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = std::fs::remove_file(&pidfile);
            Ok(())
        }
        Some(name) => Err(StopError::NotAFleetDaemon { pid, name }),
    }
}

/// `POST /api/shutdown`; true when the daemon accepted it. False for no answer or another status (an older daemon).
fn ask_to_stop(base_url: &str) -> bool {
    let url = format!("{}/api/shutdown", base_url.trim_end_matches('/'));
    ureq::post(&url)
        .config()
        .timeout_global(Some(Duration::from_secs(2)))
        .build()
        .send_empty()
        .is_ok_and(|r| r.status() == 202)
}

/// The fallback when nothing answers on the port: SIGTERM, which every daemon handles.
#[cfg(unix)]
fn signal_stop(pid: u32) -> Result<(), StopError> {
    // SAFETY: plain kill(2) on a pid we just identified; no memory is involved.
    if unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) } != 0 {
        return Err(StopError::Signal(
            std::io::Error::last_os_error().to_string(),
        ));
    }
    Ok(())
}

/// Windows has no gentle signal for a detached process: a daemon that does not answer `/api/shutdown` is reported,
/// not killed.
#[cfg(not(unix))]
fn signal_stop(pid: u32) -> Result<(), StopError> {
    Err(StopError::Signal(format!(
        "pid {pid} did not accept POST /api/shutdown; stop it from Task Manager"
    )))
}

#[cfg(test)]
mod name_tests {
    use super::is_fleet_daemon;

    #[test]
    fn accepts_the_plain_name_a_path_and_dist_names_cut_by_comm() {
        for name in [
            "fleet-daemon",
            "/x/fleet-daemon",
            "fleet-daemon-x8",
            "fleet-daemon-x86_64-linux",
        ] {
            assert!(is_fleet_daemon(name), "{name}");
        }
    }

    #[test]
    fn rejects_other_names() {
        for name in ["fleet-daemonx", "bash", "fleet-gui"] {
            assert!(!is_fleet_daemon(name), "{name}");
        }
    }
}
