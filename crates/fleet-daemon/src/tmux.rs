//! Thin wrapper over the `tmux` binary. Every call is argv-based, never through a shell, and
//! targets sessions by exact name (`=name`) so `fleet-ab` can never hit `fleet-abc`.
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

pub const MIN_VERSION: (u32, u32) = (3, 2);

#[derive(Debug, thiserror::Error)]
pub enum TmuxError {
    #[error("cannot run tmux: {0}")]
    Io(#[from] std::io::Error),
    #[error("tmux {args} failed: {stderr}")]
    Failed { args: String, stderr: String },
    #[error("tmux {found} is too old; fleet-manager needs {}.{} or newer", MIN_VERSION.0, MIN_VERSION.1)]
    Unsupported { found: String },
}

type Result<T> = std::result::Result<T, TmuxError>;

/// Parses `tmux -V` output such as `tmux 3.8`, `tmux 3.3a` or `tmux next-3.5`.
pub fn parse_version(text: &str) -> Option<(u32, u32)> {
    let v = text.trim().strip_prefix("tmux")?.trim().trim_start_matches("next-");
    let (major, rest) = v.split_once('.')?;
    let minor: String = rest.chars().take_while(char::is_ascii_digit).collect();
    Some((major.parse().ok()?, minor.parse().ok()?))
}

pub fn is_supported(v: (u32, u32)) -> bool {
    v >= MIN_VERSION
}

/// Pane commands need `=name:` (exact session, its active pane); bare `=name` is rejected by tmux.
fn pane(name: &str) -> String {
    format!("={name}:")
}

#[derive(Clone, Debug)]
pub struct Tmux {
    /// `-L` socket name; `Some` selects a dedicated tmux server (used by tests and, by default, the daemon).
    socket: Option<String>,
}

impl Tmux {
    pub fn new(socket: Option<String>) -> Tmux {
        Tmux { socket }
    }

    fn command(&self) -> Command {
        let mut c = Command::new("tmux");
        c.arg("-u");
        if let Some(s) = &self.socket {
            c.arg("-L").arg(s);
        }
        c.stdin(Stdio::null());
        c
    }

    async fn run(&self, args: &[&str]) -> Result<String> {
        let out = self.command().args(args).output().await?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(TmuxError::Failed {
                args: args.join(" "),
                stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
            })
        }
    }

    pub async fn ensure_supported(&self) -> Result<()> {
        let out = Command::new("tmux").arg("-V").output().await?;
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        match parse_version(&text) {
            Some(v) if is_supported(v) => Ok(()),
            _ => Err(TmuxError::Unsupported { found: text }),
        }
    }

    pub async fn new_session(&self, name: &str, cwd: &Path, env: &[(&str, &str)], argv: &[String]) -> Result<()> {
        let cwd = cwd.to_string_lossy().into_owned();
        let mut args: Vec<String> =
            ["new-session", "-d", "-s", name, "-c", &cwd, "-x", "200", "-y", "50"].map(String::from).to_vec();
        for (k, v) in env {
            args.push("-e".into());
            args.push(format!("{k}={v}"));
        }
        if !argv.is_empty() {
            args.push("--".into());
            args.extend(argv.iter().cloned());
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        self.run(&refs).await.map(drop)
    }

    /// Types `text` into the pane as literal characters, then presses Enter.
    pub async fn send_literal(&self, name: &str, text: &str) -> Result<()> {
        let target = pane(name);
        self.run(&["send-keys", "-t", &target, "-l", "--", text]).await?;
        self.run(&["send-keys", "-t", &target, "Enter"]).await.map(drop)
    }

    pub async fn send_key(&self, name: &str, key: &str) -> Result<()> {
        self.run(&["send-keys", "-t", &pane(name), key]).await.map(drop)
    }

    pub async fn capture(&self, name: &str) -> Result<String> {
        self.run(&["capture-pane", "-p", "-J", "-t", &pane(name)]).await
    }

    pub async fn has_session(&self, name: &str) -> Result<bool> {
        let out = self.command().args(["has-session", "-t", &format!("={name}")]).output().await?;
        Ok(out.status.success())
    }

    pub async fn list_sessions(&self, prefix: &str) -> Result<Vec<String>> {
        match self.run(&["list-sessions", "-F", "#{session_name}"]).await {
            Ok(out) => Ok(out.lines().filter(|l| l.starts_with(prefix)).map(String::from).collect()),
            // No server means no sessions; that is a normal state after a reboot.
            Err(TmuxError::Failed { stderr, .. }) if stderr.contains("no server running") || stderr.contains("No such file") => Ok(vec![]),
            Err(e) => Err(e),
        }
    }

    pub async fn kill(&self, name: &str) -> Result<()> {
        self.run(&["kill-session", "-t", &format!("={name}")]).await.map(drop)
    }

    /// For test cleanup in `Drop`; ignores failure because the server may already be gone.
    /// Also unlinks the socket file, which tmux leaves behind when the server is killed this way.
    pub fn kill_server_blocking(&self) {
        let mut c = std::process::Command::new("tmux");
        if let Some(s) = &self.socket {
            c.arg("-L").arg(s);
        }
        let _ = c.arg("kill-server").stdin(Stdio::null()).stderr(Stdio::null()).status();
        if let Some(s) = &self.socket {
            // SAFETY: getuid(2) takes no arguments and cannot fail.
            let uid = unsafe { libc::getuid() };
            let dir = std::env::var_os("TMUX_TMPDIR").map(std::path::PathBuf::from).unwrap_or_else(|| "/tmp".into());
            let _ = std::fs::remove_file(dir.join(format!("tmux-{uid}")).join(s));
        }
    }
}
