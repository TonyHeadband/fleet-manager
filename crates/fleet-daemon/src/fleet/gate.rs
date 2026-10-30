//! The test gate `open_pr` runs before any push: the repo's workflow steps, in the orchestrator's worktree.
use crate::git;
use process_wrap::tokio::ChildWrapper;
use std::collections::VecDeque;
use std::io::Read;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How much of the output a run keeps, for the orchestrator and the side panel.
pub const TAIL_LINES: usize = 200;
/// A longer line is cut, so the tail stays small whatever the command prints.
const MAX_LINE: usize = 2000;
/// How long the output is still read once the command has ended, for a process it left behind holding the pipe.
const DRAIN: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// `None` when the command timed out.
    pub exit_code: Option<i32>,
    pub duration_secs: u64,
    pub output_tail: String,
}

/// The last `TAIL_LINES` lines read so far, the unfinished one apart.
#[derive(Default)]
struct Tail {
    lines: VecDeque<String>,
    partial: Vec<u8>,
}

impl Tail {
    fn push(&mut self, bytes: &[u8]) {
        for chunk in bytes.split_inclusive(|b| *b == b'\n') {
            let room = MAX_LINE.saturating_sub(self.partial.len());
            let (text, ended) = match chunk.strip_suffix(b"\n") {
                Some(t) => (t, true),
                None => (chunk, false),
            };
            self.partial
                .extend_from_slice(&text[..text.len().min(room)]);
            if ended {
                self.end_line();
            }
        }
    }

    fn end_line(&mut self) {
        let line = String::from_utf8_lossy(&self.partial).into_owned();
        self.partial.clear();
        if self.lines.len() == TAIL_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    fn text(&self) -> String {
        let mut lines: Vec<String> = self.lines.iter().cloned().collect();
        if !self.partial.is_empty() {
            if lines.len() == TAIL_LINES {
                lines.remove(0);
            }
            lines.push(String::from_utf8_lossy(&self.partial).into_owned());
        }
        lines.join("\n")
    }
}

/// Whether the gate keeps `name` out of a step's environment: the persona's commit identity and protocol limit
/// would meet the repo's own tests where CI has neither. One list for what `gate_vars` leaves out and what `run`
/// removes from the inherited environment.
pub fn left_out(name: &str) -> bool {
    name.starts_with("GIT_AUTHOR_")
        || name.starts_with("GIT_COMMITTER_")
        || name == "GIT_ALLOW_PROTOCOL"
}

fn remove_left_out(cmd: &mut std::process::Command) {
    for (k, _) in std::env::vars_os() {
        if k.to_str().is_some_and(left_out) {
            cmd.env_remove(k);
        }
    }
}

/// Runs `<shell> -c <command>` (`shell` a POSIX shell: `Tools::posix_shell`) in `cwd` with `env` (a persona's, on top of the daemon's own PATH and HOME, as
/// the persona checks run their CLIs; whatever `left_out` names is removed from the inherited environment first, so
/// only `env` can set it), stdin closed, stdout and stderr on one pipe. It runs in a process group
/// (Unix) or Job Object (Windows) of its own, so a timeout kills everything it started, not only the shell.
pub async fn run(
    shell: &Path,
    command: &str,
    cwd: &Path,
    env: &[(String, String)],
    timeout: Duration,
) -> std::io::Result<Outcome> {
    let (mut reader, writer) = std::io::pipe()?;
    let started = Instant::now();
    let out = writer.try_clone()?;
    let mut cmd = process_wrap::tokio::CommandWrap::with_new(shell, |c| {
        remove_left_out(c.as_std_mut());
        git::harden(c.as_std_mut(), env);
        c.arg("-c")
            .arg(command)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(writer);
    });
    // The whole tree, not only the shell: a process group on Unix, a Job Object on Windows.
    #[cfg(unix)]
    cmd.wrap(process_wrap::tokio::ProcessGroup::leader());
    #[cfg(windows)]
    cmd.wrap(process_wrap::tokio::JobObject);
    cmd.wrap(process_wrap::tokio::KillOnDrop);
    let child = cmd.spawn()?;
    // `cmd` goes here, and with it the daemon's copies of the write end: the read ends with the command's.
    drop(cmd);
    // A cancelled call, an error or a panic must end the whole tree too: `KillOnDrop` alone reaches the shell (and
    // the Job Object, which closes with it), but not a Unix process group.
    let mut child = TreeGuard::new(child);

    let tail = Arc::new(Mutex::new(Tail::default()));
    let filling = tail.clone();
    // Blocking reads on their own thread; it is left behind only if something the command started outlives it
    // outside its group and keeps the pipe open.
    let mut drained = tokio::task::spawn_blocking(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => filling.lock().expect("tail poisoned").push(&buf[..n]),
            }
        }
    });

    let exit_code = match tokio::time::timeout(timeout, wait_for_shell(child.get())).await {
        Ok(status) => {
            let status = status?;
            // Reaped: the group id may be reused from here, so the guard must not fire on it any more.
            child.disarm();
            // A signal death is not a timeout; reported as a shell would.
            #[cfg(unix)]
            let code = {
                use std::os::unix::process::ExitStatusExt;
                status
                    .code()
                    .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
            };
            #[cfg(not(unix))]
            let code = status.code().unwrap_or(1);
            Some(code)
        }
        Err(_) => {
            // Before the leader is reaped, so the group id cannot have been reused.
            let _ = child.get().start_kill();
            child.get().wait().await?;
            child.disarm();
            None
        }
    };
    let duration_secs = started.elapsed().as_secs();
    if tokio::time::timeout(DRAIN, &mut drained).await.is_err() {
        // Something left running still holds the pipe; the command is over, so it goes too.
        let _ = child.get().start_kill();
        let _ = tokio::time::timeout(DRAIN, drained).await;
    }
    let output_tail = tail.lock().expect("tail poisoned").text();
    Ok(Outcome {
        exit_code,
        duration_secs,
        output_tail,
    })
}

/// Waits for the shell itself. On Windows `JobObjectChild::wait` is meant to wait until the Job Object is empty, so a
/// background child would hold the gate up for as long as it lives (process-wrap 10.0.1 returns early, on the first
/// queued job message, but nothing promises that); it is the pipe drain that deals with those.
async fn wait_for_shell(child: &mut dyn ChildWrapper) -> std::io::Result<std::process::ExitStatus> {
    #[cfg(windows)]
    let child = child.inner_mut();
    child.wait().await
}

/// Kills the command's whole tree when dropped, unless its leader was reaped first.
struct TreeGuard {
    child: Box<dyn ChildWrapper>,
    armed: bool,
}

impl TreeGuard {
    fn new(child: Box<dyn ChildWrapper>) -> Self {
        Self { child, armed: true }
    }

    fn get(&mut self) -> &mut dyn ChildWrapper {
        &mut *self.child
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TreeGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.child.start_kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_keeps_the_last_lines_and_cuts_long_ones() {
        let mut t = Tail::default();
        for i in 1..=300 {
            t.push(format!("{i}\n").as_bytes());
        }
        t.push(b"unfin");
        t.push(b"ished");
        let text = t.text();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), TAIL_LINES);
        assert_eq!(lines[0], "102");
        assert_eq!(lines.last(), Some(&"unfinished"));

        let mut t = Tail::default();
        t.push(&vec![b'x'; MAX_LINE * 3]);
        t.push(b"\nend\n");
        assert_eq!(t.text(), format!("{}\nend", "x".repeat(MAX_LINE)));
    }

    /// Re-runs itself with the variables in the inherited environment: the test runner's own must not matter, and
    /// `set_var` is not safe next to other threads.
    #[cfg(unix)]
    #[tokio::test]
    async fn inherited_identity_and_protocol_limit_do_not_reach_the_step() {
        const MARK: &str = "FLEET_GATE_TEST_CHILD";
        const NAME: &str = "inherited_identity_and_protocol_limit_do_not_reach_the_step";
        if std::env::var_os(MARK).is_none() {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    &format!("fleet::gate::tests::{NAME}"),
                    "--test-threads=1",
                ])
                .env(MARK, "1")
                .env("GIT_AUTHOR_NAME", "a")
                .env("GIT_AUTHOR_DATE", "b")
                .env("GIT_COMMITTER_EMAIL", "c")
                .env("GIT_ALLOW_PROTOCOL", "file")
                .env("GIT_DIR_NOT_LEFT_OUT", "d")
                .status()
                .unwrap();
            assert!(status.success(), "the child run failed");
            return;
        }
        let t = tempfile::tempdir().unwrap();
        let out = run(
            Path::new("sh"),
            "env",
            t.path(),
            &[("GIT_COMMITTER_NAME".into(), "gate".into())],
            Duration::from_secs(10),
        )
        .await
        .unwrap();
        let seen: Vec<&str> = out.output_tail.lines().collect();
        assert!(
            !seen.iter().any(|l| l.starts_with("GIT_AUTHOR_")),
            "{seen:?}"
        );
        assert!(
            !seen.iter().any(|l| l.starts_with("GIT_COMMITTER_EMAIL=")),
            "{seen:?}"
        );
        assert!(
            !seen.iter().any(|l| l.starts_with("GIT_ALLOW_PROTOCOL=")),
            "{seen:?}"
        );
        assert!(
            seen.contains(&"GIT_COMMITTER_NAME=gate"),
            "the gate's own env stays: {seen:?}"
        );
        assert!(
            seen.contains(&"GIT_DIR_NOT_LEFT_OUT=d"),
            "other vars stay: {seen:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_timeout_kills_what_the_command_started() {
        let t = tempfile::tempdir().unwrap();
        let pidfile = t.path().join("child.pid");
        let cmd = format!("sleep 30 & echo $! > {}; wait", pidfile.display());
        let out = run(Path::new("sh"), &cmd, t.path(), &[], Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(out.exit_code, None, "a timeout reports no exit code");
        let pid = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .to_string();
        let alive = std::process::Command::new("kill")
            .args(["-0", &pid])
            .status()
            .unwrap()
            .success();
        assert!(!alive, "the grandchild {pid} outlived the gate");
    }

    /// A background child stays in the Job Object after the shell exits, without the output pipe (so the drain is not
    /// what ends it); the gate must not wait for it to finish.
    #[cfg(windows)]
    #[tokio::test]
    async fn a_background_child_does_not_hold_the_gate_up() {
        let t = tempfile::tempdir().unwrap();
        let started = Instant::now();
        let out = run(
            &fleet_fs::posix_shell().unwrap(),
            "sleep 30 >/dev/null 2>&1 </dev/null &",
            t.path(),
            &[],
            Duration::from_secs(20),
        )
        .await
        .unwrap();
        assert_eq!(out.exit_code, Some(0));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "took {:?}",
            started.elapsed()
        );
    }
}
