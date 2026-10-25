//! The test gate `open_pr` runs before any push: the repo's test command, in the orchestrator's worktree.
use crate::git;
use std::collections::VecDeque;
use std::io::Read;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::process::Command;

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

/// Runs `sh -c <command>` in `cwd` with `env` (a persona's, on top of the daemon's own PATH and HOME, as the
/// persona checks run their CLIs), stdin closed, stdout and stderr on one pipe. It runs in a process group of its
/// own, so a timeout kills everything it started, not only the shell.
pub async fn run(
    command: &str,
    cwd: &Path,
    env: &[(String, String)],
    timeout: Duration,
) -> std::io::Result<Outcome> {
    let (mut reader, writer) = std::io::pipe()?;
    let started = Instant::now();
    let mut child = {
        let mut cmd = Command::new("sh");
        git::harden(cmd.as_std_mut(), env);
        cmd.arg("-c")
            .arg(command)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(writer.try_clone()?)
            .stderr(writer)
            .process_group(0)
            .kill_on_drop(true);
        cmd.spawn()?
        // `cmd` goes here, and with it the daemon's copies of the write end: the read ends with the command's.
    };
    let group = child.id().and_then(|p| i32::try_from(p).ok());
    // `kill_on_drop` reaches only the shell: a cancelled call, an error or a panic must end the whole group too.
    let mut armed = GroupGuard(group);

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

    let exit_code = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(status) => {
            let status = status?;
            // Reaped: the group id may be reused from here, so the guard must not fire on it any more.
            armed.disarm();
            // A signal death is not a timeout; reported as a shell would.
            Some(
                status
                    .code()
                    .unwrap_or_else(|| 128 + status.signal().unwrap_or(0)),
            )
        }
        Err(_) => {
            // Before the leader is reaped, so the group id cannot have been reused.
            if let Some(g) = group {
                kill_group(g);
            }
            child.wait().await?;
            armed.disarm();
            None
        }
    };
    let duration_secs = started.elapsed().as_secs();
    if tokio::time::timeout(DRAIN, &mut drained).await.is_err() {
        // Something left running still holds the pipe; the command is over, so it goes too.
        if let Some(g) = group {
            kill_group(g);
        }
        let _ = tokio::time::timeout(DRAIN, drained).await;
    }
    let output_tail = tail.lock().expect("tail poisoned").text();
    Ok(Outcome {
        exit_code,
        duration_secs,
        output_tail,
    })
}

/// Kills the command's process group when dropped, unless its leader was reaped first.
struct GroupGuard(Option<i32>);

impl GroupGuard {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for GroupGuard {
    fn drop(&mut self) {
        if let Some(g) = self.0 {
            kill_group(g);
        }
    }
}

fn kill_group(group: i32) {
    // SAFETY: killpg only sends a signal; a group that is gone gives ESRCH, which is fine.
    unsafe {
        libc::killpg(group, libc::SIGKILL);
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
}
