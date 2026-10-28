//! Thin wrapper over the `tmux` binary. Every call is argv-based, never through a shell, and
//! targets sessions by exact name (`=name`) so `fleet-ab` can never hit `fleet-abc`.
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

pub const MIN_VERSION: (u32, u32) = (3, 2);

#[derive(Debug, thiserror::Error)]
pub enum TmuxError {
    #[error("cannot run {program}: {source}")]
    Io {
        program: String,
        source: std::io::Error,
    },
    #[error("tmux {args} failed: {stderr}")]
    Failed { args: String, stderr: String },
    #[error("{program} reports {found:?}; fleet-manager needs tmux {}.{} or newer (tmux_command in config.toml can name another one)", MIN_VERSION.0, MIN_VERSION.1)]
    Unsupported { program: String, found: String },
}

type Result<T> = std::result::Result<T, TmuxError>;

/// Parses `tmux -V` output such as `tmux 3.8`, `tmux 3.3a` or `tmux next-3.5`.
pub fn parse_version(text: &str) -> Option<(u32, u32)> {
    let v = text
        .trim()
        .strip_prefix("tmux")?
        .trim()
        .trim_start_matches("next-");
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
    /// The binary: `tmux` from PATH unless the config's `tmux_command` names another.
    program: String,
}

impl Tmux {
    pub fn new(socket: Option<String>) -> Tmux {
        Tmux {
            socket,
            program: "tmux".into(),
        }
    }

    pub fn with_program(mut self, program: impl Into<String>) -> Tmux {
        self.program = program.into();
        self
    }

    /// The argv that attaches a terminal to `session`. It names the same program and `-L` socket the daemon uses, so a
    /// client on this machine reaches the daemon's tmux server and not the user's own. `-u` as in every other
    /// command: without it a non-UTF-8 locale draws Claude's box and glyph characters as `_`.
    pub fn attach_argv(&self, session: &str) -> Vec<String> {
        let mut argv = vec![self.program.clone(), "-u".to_string()];
        if let Some(s) = &self.socket {
            argv.extend(["-L".to_string(), s.clone()]);
        }
        argv.extend(["attach".to_string(), "-t".to_string(), session.to_string()]);
        argv
    }

    fn io(&self, source: std::io::Error) -> TmuxError {
        TmuxError::Io {
            program: self.program.clone(),
            source,
        }
    }

    fn command(&self) -> Command {
        let mut c = Command::new(&self.program);
        c.arg("-u");
        if let Some(s) = &self.socket {
            c.arg("-L").arg(s);
        }
        c.stdin(Stdio::null());
        c
    }

    async fn run(&self, args: &[&str]) -> Result<String> {
        let out = self
            .command()
            .args(args)
            .output()
            .await
            .map_err(|e| self.io(e))?;
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
        let out = Command::new(&self.program)
            .arg("-V")
            .output()
            .await
            .map_err(|e| self.io(e))?;
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        match parse_version(&text) {
            Some(v) if is_supported(v) => Ok(()),
            _ => Err(TmuxError::Unsupported {
                program: self.program.clone(),
                found: text,
            }),
        }
    }

    pub async fn new_session(
        &self,
        name: &str,
        cwd: &Path,
        env: &[(&str, &str)],
        argv: &[String],
    ) -> Result<()> {
        let cwd = cwd.to_string_lossy().into_owned();
        let mut args: Vec<String> = [
            "new-session",
            "-d",
            "-s",
            name,
            "-c",
            &cwd,
            "-x",
            "200",
            "-y",
            "50",
        ]
        .map(String::from)
        .to_vec();
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

    /// Keeps `vars` out of every later process in the session: a window opened in it would otherwise take them
    /// from the server's global environment (a server started from a shell with tokens keeps them), and an attaching
    /// client would copy those named in `update-environment` back in. One tmux call, `;`-separated.
    pub async fn drop_environment(&self, name: &str, vars: &[&str]) -> Result<()> {
        let session = format!("={name}");
        let updated = self
            .run(&["show-options", "-gv", "update-environment"])
            .await?;
        let kept: Vec<&str> = updated
            .split_whitespace()
            .filter(|v| !vars.contains(v))
            .collect();
        let kept = kept.join(" ");
        // `-r`, not `-u`: unsetting the session's copy would fall back to the global value.
        let mut args = Vec::new();
        for v in vars {
            args.extend(["set-environment", "-t", &session, "-r", v, ";"]);
        }
        let target = pane(name);
        args.extend(["set-option", "-t", &target, "update-environment", &kept]);
        self.run(&args).await.map(drop)
    }

    /// Leaves any mode the pane is in. A person who scrolled it from an attached terminal leaves it in copy mode,
    /// where sent keys run copy-mode bindings instead of reaching the program. Best effort: a tmux that refuses
    /// `-q` sends as before.
    async fn leave_mode(&self, target: &str) {
        let _ = self.run(&["copy-mode", "-q", "-t", target]).await;
    }

    /// Types `text` into the pane as literal characters, then presses Enter.
    pub async fn send_literal(&self, name: &str, text: &str) -> Result<()> {
        let target = pane(name);
        self.leave_mode(&target).await;
        self.run(&["send-keys", "-t", &target, "-l", "--", text])
            .await?;
        self.run(&["send-keys", "-t", &target, "Enter"])
            .await
            .map(drop)
    }

    pub async fn send_key(&self, name: &str, key: &str) -> Result<()> {
        let target = pane(name);
        self.leave_mode(&target).await;
        self.run(&["send-keys", "-t", &target, key]).await.map(drop)
    }

    pub async fn capture(&self, name: &str) -> Result<String> {
        self.run(&["capture-pane", "-p", "-J", "-t", &pane(name)])
            .await
    }

    pub async fn has_session(&self, name: &str) -> Result<bool> {
        let out = self
            .command()
            .args(["has-session", "-t", &format!("={name}")])
            .output()
            .await
            .map_err(|e| self.io(e))?;
        Ok(out.status.success())
    }

    pub async fn list_sessions(&self, prefix: &str) -> Result<Vec<String>> {
        match self.run(&["list-sessions", "-F", "#{session_name}"]).await {
            Ok(out) => Ok(out
                .lines()
                .filter(|l| l.starts_with(prefix))
                .map(String::from)
                .collect()),
            // No server means no sessions; that is a normal state after a reboot.
            Err(TmuxError::Failed { stderr, .. })
                if stderr.contains("no server running") || stderr.contains("No such file") =>
            {
                Ok(vec![])
            }
            Err(e) => Err(e),
        }
    }

    pub async fn kill(&self, name: &str) -> Result<()> {
        self.run(&["kill-session", "-t", &format!("={name}")])
            .await
            .map(drop)
    }

    /// For test cleanup in `Drop`; ignores failure because the server may already be gone.
    /// Also unlinks the socket file, which tmux leaves behind when the server is killed this way.
    pub fn kill_server_blocking(&self) {
        let mut c = std::process::Command::new(&self.program);
        if let Some(s) = &self.socket {
            c.arg("-L").arg(s);
        }
        let _ = c
            .arg("kill-server")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .status();
        #[cfg(unix)]
        if let Some(s) = &self.socket {
            // SAFETY: getuid(2) takes no arguments and cannot fail.
            let uid = unsafe { libc::getuid() };
            let dir = std::env::var_os("TMUX_TMPDIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| "/tmp".into());
            let _ = std::fs::remove_file(dir.join(format!("tmux-{uid}")).join(s));
        }
    }
}
