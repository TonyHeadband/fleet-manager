//! Plays a scenario like Claude Code would: hook posts to the daemon plus a transcript on the pane.
use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use serde_json::{json, Value};

use crate::hooks::{payload, Hook, Sink};
use crate::input::{shutdown_requested, Input};
use crate::scenario::{Scenario, Step, StepKind};

// Claude Code's own markers (⏺, ⎿, ✻, ❯) are missing from IBM Plex Mono, the GUI's terminal font, and would show
// as empty boxes in the demo; these stand-ins are in it (checked by `transcript_uses_only_glyphs_the_gui_font_has`).

/// Granularity at which a scripted delay notices Escape or a shutdown.
const TICK: Duration = Duration::from_millis(100);

enum Stop {
    /// A line typed in place of the awaited answer; idle handling treats it as a fresh prompt.
    Redirect(String),
    End,
    Fail(String),
}

impl From<String> for Stop {
    fn from(e: String) -> Self {
        Stop::Fail(e)
    }
}

type Flow = Result<(), Stop>;

pub struct Player<S: Sink, W: Write> {
    scenario: Scenario,
    sink: S,
    out: W,
    cwd: PathBuf,
    session_id: String,
    scenario_dir: PathBuf,
    sleep: fn(Duration),
    /// Input that arrived while the player was not waiting for it.
    pending: VecDeque<Input>,
    closed: bool,
}

impl<S: Sink, W: Write> Player<S, W> {
    pub fn new(
        scenario: Scenario,
        sink: S,
        out: W,
        cwd: PathBuf,
        session_id: String,
        scenario_dir: PathBuf,
        sleep: fn(Duration),
    ) -> Self {
        Self {
            scenario,
            sink,
            out,
            cwd,
            session_id,
            scenario_dir,
            sleep,
            pending: VecDeque::new(),
            closed: false,
        }
    }

    pub fn sink(&self) -> &S {
        &self.sink
    }

    pub fn run(&mut self, input: &Receiver<Input>) -> Result<(), String> {
        let result = self.play(input);
        // Best effort: the daemon may already be gone when the pane is being torn down.
        self.post(Hook::SessionEnd);
        match result {
            Err(Stop::Fail(e)) => Err(e),
            _ => Ok(()),
        }
    }

    fn play(&mut self, rx: &Receiver<Input>) -> Flow {
        self.post(Hook::SessionStart);
        let prompt = self.scenario.task.clone();
        self.post(Hook::UserPromptSubmit {
            prompt: prompt.clone(),
        });
        self.say("* Welcome to Claude Code!\n");
        self.say(&format!("> {prompt}\n"));
        let main = self.scenario.main.clone();
        let mut carry = match self.steps(&main, rx) {
            Ok(()) => None,
            Err(Stop::Redirect(l)) => Some(l),
            Err(e) => return Err(e),
        };
        loop {
            let line = match carry.take() {
                Some(l) => l,
                None => match self.wait_line(rx) {
                    Ok(l) => l,
                    Err(Stop::Redirect(l)) => l,
                    Err(e) => return Err(e),
                },
            };
            match self.prompt(&line, rx) {
                Ok(()) => {}
                Err(Stop::Redirect(l)) => carry = Some(l),
                Err(e) => return Err(e),
            }
        }
    }

    fn prompt(&mut self, line: &str, rx: &Receiver<Input>) -> Flow {
        self.say(&format!("> {line}\n"));
        self.post(Hook::UserPromptSubmit {
            prompt: line.to_string(),
        });
        let follow = self
            .scenario
            .follow_ups
            .iter()
            .find(|f| f.on.trim() == line.trim())
            .map(|f| f.steps.clone());
        match follow {
            Some(steps) => self.steps(&steps, rx),
            None => {
                self.post(Hook::Stop);
                Ok(())
            }
        }
    }

    fn steps(&mut self, steps: &[Step], rx: &Receiver<Input>) -> Flow {
        for step in steps {
            self.delay(step.after, rx)?;
            match &step.kind {
                StepKind::Say(text) => self.say(&format!("• {text}\n")),
                StepKind::Tool {
                    tool,
                    input,
                    output,
                } => {
                    let args = tool_input(tool, input);
                    self.post(Hook::PreToolUse {
                        tool: tool.clone(),
                        input: args.clone(),
                    });
                    self.say(&format!("• {tool}({input})\n"));
                    for (i, l) in output.iter().enumerate() {
                        self.say(&format!("{}{l}\n", if i == 0 { "  └  " } else { "     " }));
                    }
                    self.post(Hook::PostToolUse {
                        tool: tool.clone(),
                        input: args,
                    });
                }
                StepKind::Permission { tool, input } => {
                    let args = tool_input(tool, input);
                    self.post(Hook::PermissionRequest {
                        tool: tool.clone(),
                        input: args.clone(),
                    });
                    self.permission_box(tool, input);
                    let line = self.wait_line(rx)?;
                    self.say(&format!("> {line}\n"));
                    self.post(Hook::PostToolUse {
                        tool: tool.clone(),
                        input: args,
                    });
                }
                StepKind::Question { text } => {
                    self.post(Hook::Question { text: text.clone() });
                    self.say(&format!("• {text}\n"));
                    let line = self.wait_line(rx)?;
                    self.say(&format!("> {line}\n"));
                    self.post(Hook::UserPromptSubmit { prompt: line });
                }
                StepKind::Write { path, from, commit } => {
                    self.write_file(path, from, commit.as_deref())?
                }
                StepKind::Stop => self.post(Hook::Stop),
            }
        }
        Ok(())
    }

    fn write_file(&mut self, path: &str, from: &str, commit: Option<&str>) -> Flow {
        let repo = repo_dir(&self.cwd);
        let args = json!({ "file_path": path });
        self.post(Hook::PreToolUse {
            tool: "Write".into(),
            input: args.clone(),
        });
        let dest = repo.join(path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        let src = self.scenario_dir.join(from);
        std::fs::copy(&src, &dest)
            .map_err(|e| format!("cannot copy {} to {}: {e}", src.display(), dest.display()))?;
        self.say(&format!("• Write({path})\n"));
        if let Some(message) = commit {
            git(&repo, &["add", "--", path])?;
            git(&repo, &["commit", "-q", "-m", message])?;
        }
        self.post(Hook::PostToolUse {
            tool: "Write".into(),
            input: args,
        });
        Ok(())
    }

    /// Sleeps in ticks so Escape and shutdown are noticed; the clock does not run while paused.
    fn delay(&mut self, total: Duration, rx: &Receiver<Input>) -> Flow {
        let mut remaining = total;
        loop {
            if shutdown_requested() {
                return Err(Stop::End);
            }
            self.drain(rx);
            if let Some(pos) = self.pending.iter().position(|i| *i == Input::Escape) {
                self.pending.remove(pos);
                self.interrupted();
                // Lines queued before the Escape are answers for a later wait, not the resume line.
                let line = self.next_line_from(pos, rx)?;
                if line.trim() != "continue" {
                    return Err(Stop::Redirect(line));
                }
            }
            if remaining.is_zero() {
                return Ok(());
            }
            let slice = remaining.min(TICK);
            (self.sleep)(slice);
            remaining -= slice;
        }
    }

    fn drain(&mut self, rx: &Receiver<Input>) {
        use std::sync::mpsc::TryRecvError;
        loop {
            match rx.try_recv() {
                Ok(i) => self.pending.push_back(i),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.closed = true;
                    return;
                }
            }
        }
    }

    fn recv(&mut self, rx: &Receiver<Input>) -> Input {
        if let Some(i) = self.pending.pop_front() {
            return i;
        }
        if self.closed {
            return Input::Eof;
        }
        rx.recv().unwrap_or_else(|_| {
            self.closed = true;
            Input::Eof
        })
    }

    /// The next answer. Escape pauses; `continue` keeps waiting, any other line replaces the wait.
    fn wait_line(&mut self, rx: &Receiver<Input>) -> Result<String, Stop> {
        loop {
            match self.recv(rx) {
                Input::Line(l) => return Ok(l),
                Input::Escape => {
                    self.interrupted();
                    let line = self.next_line(rx)?;
                    if line.trim() != "continue" {
                        return Err(Stop::Redirect(line));
                    }
                }
                Input::Eof => return Err(Stop::End),
            }
        }
    }

    fn next_line_from(&mut self, from: usize, rx: &Receiver<Input>) -> Result<String, Stop> {
        while from < self.pending.len() {
            match self.pending.remove(from) {
                Some(Input::Line(l)) => return Ok(l),
                Some(Input::Eof) => return Err(Stop::End),
                _ => {}
            }
        }
        loop {
            let next = if self.closed {
                Input::Eof
            } else {
                rx.recv().unwrap_or_else(|_| {
                    self.closed = true;
                    Input::Eof
                })
            };
            match next {
                Input::Line(l) => return Ok(l),
                Input::Escape => {}
                Input::Eof => return Err(Stop::End),
            }
        }
    }

    fn next_line(&mut self, rx: &Receiver<Input>) -> Result<String, Stop> {
        loop {
            match self.recv(rx) {
                Input::Line(l) => return Ok(l),
                Input::Escape => {}
                Input::Eof => return Err(Stop::End),
            }
        }
    }

    fn interrupted(&mut self) {
        self.say("  └  Interrupted · What should Claude do instead?\n");
    }

    fn permission_box(&mut self, tool: &str, input: &str) {
        let rule = "─".repeat(60);
        self.say(&format!(
            "╭{rule}╮\n│ {tool} command\n│\n│   {input}\n│\n│ Do you want to proceed?\n│ › 1. Yes\n│   2. No, and tell Claude what to do differently\n╰{rule}╯\n"
        ));
    }

    fn say(&mut self, text: &str) {
        let _ = self.out.write_all(text.as_bytes());
        let _ = self.out.flush();
    }

    fn post(&mut self, hook: Hook) {
        let body = payload(&hook, &self.session_id, &self.cwd);
        self.sink.send(body);
    }
}

/// Shapes the scenario's single string like the real tool's arguments, which is what the reducer reads.
fn tool_input(tool: &str, input: &str) -> Value {
    match tool {
        "Bash" => json!({ "command": input }),
        "Read" | "Edit" | "Write" => json!({ "file_path": input }),
        _ => json!({ "input": input }),
    }
}

/// The task folder holds one worktree per repo; commits belong in that worktree, not the folder.
fn repo_dir(cwd: &Path) -> PathBuf {
    let mut repos = std::fs::read_dir(cwd)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join(".git").exists());
    match (repos.next(), repos.next()) {
        (Some(only), None) => only,
        _ => cwd.to_path_buf(),
    }
}

fn git(repo: &Path, args: &[&str]) -> Result<(), String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}
