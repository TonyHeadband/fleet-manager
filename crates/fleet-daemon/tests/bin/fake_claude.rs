//! Test double for the claude CLI. Reports how it was started, then echoes keystrokes one at a time in `cat -v`
//! style (Escape as `^[`, Ctrl-U as `^U`) so tests can see exactly what the daemon typed. Like ConPTY at startup it
//! first asks where the cursor is and prints nothing until the terminal answers.
//!
//! Like the real CLI it turns bracketed paste on after `ready` and treats fast input as a paste. Newlines inside a
//! paste (between `ESC[200~` and `ESC[201~`), and an Enter arriving within `PASTE_GAP` of the previous byte, are
//! text: they show as `^J` / `^M` and never submit. An Enter straight after the end of a paste is a key press and
//! submits. The paste markers themselves echo like any other escape sequence (`^[[200~`, `^[[201~`).
//!
//! FAKE_CLAUDE_READY_AFTER_MS holds `ready` back that long and drops whatever was typed meanwhile, as the real TUI
//! loses keys typed before its input box is up.
//!
//! It keeps an input buffer of several lines with a cursor, edited as the real CLI's is: Down/Up move between
//! lines, Ctrl-E goes to the end of the line, Ctrl-U deletes back to the start of the line (at the start it joins
//! the line above), Backspace deletes a character (at the start it joins the line above). Ctrl-L does nothing. A
//! submitting Enter prints the whole buffer as `entry<<...>>`, newlines as `^J`, and empties it.
use std::io::{Read, Write};
use std::time::{Duration, Instant};

/// An Enter closer than this to the previous byte is part of a paste.
const PASTE_GAP: Duration = Duration::from_millis(30);
const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // FAKE_CLAUDE_VERSION sets the line.
    if args.first().is_some_and(|a| a == "--version") {
        let line = std::env::var("FAKE_CLAUDE_VERSION")
            .unwrap_or_else(|_| "2.0.14 (Claude Code)".to_string());
        println!("{line}");
        std::process::exit(0);
    }
    if args.len() >= 2 && args[0] == "auth" && args[1] == "status" {
        let logged_in = std::env::var_os("CLAUDE_CONFIG_DIR")
            .is_some_and(|d| std::path::Path::new(&d).join(".fake-login").exists());
        if logged_in {
            println!(r#"{{"loggedIn":true,"email":"tester@example.com"}}"#);
            std::process::exit(0);
        }
        println!(r#"{{"loggedIn":false}}"#);
        std::process::exit(1);
    }
    raw::enable();
    let mut out = std::io::stdout().lock();
    let mut input = std::io::stdin().lock();
    if std::env::var_os("FAKE_CLAUDE_NO_DSR").is_none() {
        ask_cursor(&mut out, &mut input);
    }
    let v = |k: &str| std::env::var(k).unwrap_or_default();
    let or_unset = |k: &str| std::env::var(k).unwrap_or_else(|_| "unset".into());
    let cwd = fleet_fs::canonical(&std::env::current_dir().unwrap()).unwrap();
    line(&mut out, &format!("cwd={}", cwd.display()));
    line(&mut out, &format!("token={}", v("FLEET_TOKEN")));
    line(
        &mut out,
        &format!(
            "mcp_timeout={} mcp_idle={} mcp_background={} term={}",
            v("MCP_TOOL_TIMEOUT"),
            v("CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT"),
            v("CLAUDE_CODE_MCP_AUTO_BACKGROUND_MS"),
            or_unset("TERM")
        ),
    );
    line(
        &mut out,
        &format!(
            "claude_config={} gh_config={} gh_host={} author={}",
            v("CLAUDE_CONFIG_DIR"),
            v("GH_CONFIG_DIR"),
            v("GH_HOST"),
            v("GIT_AUTHOR_EMAIL")
        ),
    );
    line(
        &mut out,
        &format!(
            "gh_token={} anthropic_key={} git_params={} ssh_askpass={} ssh_auth_sock={} git_askpass=[{}]",
            or_unset("GH_TOKEN"),
            or_unset("ANTHROPIC_API_KEY"),
            or_unset("GIT_CONFIG_PARAMETERS"),
            or_unset("SSH_ASKPASS"),
            or_unset("SSH_AUTH_SOCK"),
            or_unset("GIT_ASKPASS")
        ),
    );
    line(
        &mut out,
        &format!(
            "claudecode={} child_session={} session_id={}",
            or_unset("CLAUDECODE"),
            or_unset("CLAUDE_CODE_CHILD_SESSION"),
            or_unset("CLAUDE_CODE_SESSION_ID")
        ),
    );
    let value_2 = std::env::var("GIT_CONFIG_VALUE_2")
        .map(|x| format!("={x}"))
        .unwrap_or_default();
    line(
        &mut out,
        &format!(
            "git_config_count={} git_config_2={}{value_2}",
            or_unset("GIT_CONFIG_COUNT"),
            or_unset("GIT_CONFIG_KEY_2")
        ),
    );
    for a in &args {
        line(&mut out, &format!("arg={a}"));
    }
    // Like the real CLI, a new session leaves a transcript under the config dir for a later `--resume`.
    if let (Some(dir), Some(i)) = (
        std::env::var_os("CLAUDE_CONFIG_DIR"),
        args.iter().position(|a| a == "--session-id"),
    ) {
        if let Some(id) = args.get(i + 1) {
            let dir = std::path::Path::new(&dir).join("projects").join("fake-cwd");
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(dir.join(format!("{id}.jsonl")), "{}\n");
        }
    }
    if let Some(ms) = std::env::var("FAKE_CLAUDE_READY_AFTER_MS")
        .ok()
        .and_then(|m| m.parse().ok())
    {
        std::thread::sleep(Duration::from_millis(ms));
        raw::drop_input();
    }
    // Tests wait for this line: input sent before raw mode was on could have been flushed.
    line(&mut out, "ready");
    if let Ok(code) = std::env::var("FAKE_CLAUDE_EXIT") {
        // FAKE_CLAUDE_EXIT_AFTER_MS holds the exit back, so a test can act while the session is still alive.
        if let Some(ms) = std::env::var("FAKE_CLAUDE_EXIT_AFTER_MS")
            .ok()
            .and_then(|m| m.parse().ok())
        {
            std::thread::sleep(Duration::from_millis(ms));
        }
        std::process::exit(code.parse().unwrap_or(1));
    }
    if let Ok(n) = std::env::var("FAKE_CLAUDE_BURST") {
        for i in 1..=n.parse::<u32>().unwrap_or(0) {
            line(&mut out, &format!("burst {i}"));
        }
    }
    let _ = out.write_all(b"\x1b[?2004h");
    let _ = out.flush();
    let mut b = [0u8; 1];
    let mut last_byte: Option<Instant> = None;
    let mut in_paste = false;
    let mut recent: Vec<u8> = Vec::new();
    let mut escape: Vec<u8> = Vec::new();
    let mut after_paste = false;
    let mut buffer = Buffer::default();
    while input.read(&mut b).map(|n| n == 1).unwrap_or(false) {
        let fast = last_byte.is_some_and(|t| t.elapsed() < PASTE_GAP);
        last_byte = Some(Instant::now());
        recent.push(b[0]);
        if recent.len() > PASTE_START.len() {
            recent.remove(0);
        }
        let mut ended_paste = false;
        if recent == PASTE_START {
            in_paste = true;
        } else if recent == PASTE_END {
            in_paste = false;
            ended_paste = true;
        }
        let was_after_paste = std::mem::replace(&mut after_paste, ended_paste);
        let submits = matches!(b[0], b'\r' | b'\n') && !in_paste && (!fast || was_after_paste);
        let shown = match b[0] {
            _ if submits => "\r\n".to_string(),
            0x7f => "^?".to_string(),
            c if c < 0x20 => format!("^{}", (c + 0x40) as char),
            c => (c as char).to_string(),
        };
        let _ = out.write_all(shown.as_bytes());
        if submits {
            let _ = write!(out, "entry<<{}>>\r\n", buffer.take());
        } else if b[0] == 0x1b {
            escape = vec![0x1b];
        } else if escape.len() == 1 && b[0] == b'[' {
            escape.push(b[0]);
        } else if escape.len() > 1 {
            escape.push(b[0]);
            // Parameter bytes up to a final byte.
            if (0x40..=0x7e).contains(&b[0]) {
                match escape.as_slice() {
                    b"\x1b[A" => buffer.up(),
                    b"\x1b[B" => buffer.down(),
                    _ => {}
                }
                escape.clear();
            }
        } else {
            escape.clear();
            match b[0] {
                b'\r' | b'\n' => buffer.newline(),
                0x05 => buffer.end(),
                0x15 => buffer.kill_to_line_start(),
                0x7f => buffer.backspace(),
                c if c >= 0x20 => buffer.insert(c),
                _ => {}
            }
        }
        let _ = out.flush();
    }
}

/// The multi-line input box: lines of bytes and the cursor in it.
#[derive(Default)]
struct Buffer {
    lines: Vec<Vec<u8>>,
    line: usize,
    col: usize,
}

impl Buffer {
    fn current(&mut self) -> &mut Vec<u8> {
        if self.lines.is_empty() {
            self.lines.push(Vec::new());
        }
        &mut self.lines[self.line]
    }

    fn insert(&mut self, c: u8) {
        let col = self.col;
        self.current().insert(col, c);
        self.col += 1;
    }

    fn newline(&mut self) {
        let col = self.col;
        let rest = self.current().split_off(col);
        self.line += 1;
        self.lines.insert(self.line, rest);
        self.col = 0;
    }

    fn end(&mut self) {
        self.col = self.current().len();
    }

    fn up(&mut self) {
        self.line = self.line.saturating_sub(1);
        self.col = self.col.min(self.current().len());
    }

    fn down(&mut self) {
        if self.line + 1 < self.lines.len() {
            self.line += 1;
        }
        self.col = self.col.min(self.current().len());
    }

    /// Appends the current line to the one above and puts the cursor at the join.
    fn join_up(&mut self) {
        let tail = self.lines.remove(self.line);
        self.line -= 1;
        self.col = self.lines[self.line].len();
        self.lines[self.line].extend(tail);
    }

    fn kill_to_line_start(&mut self) {
        if self.col > 0 {
            let col = self.col;
            self.current().drain(..col);
            self.col = 0;
        } else if self.line > 0 {
            self.join_up();
        }
    }

    fn backspace(&mut self) {
        if self.col > 0 {
            let col = self.col;
            self.current().remove(col - 1);
            self.col -= 1;
        } else if self.line > 0 {
            self.join_up();
        }
    }

    /// The entry a submit sends, with `^J` for the line breaks; the buffer is empty afterwards.
    fn take(&mut self) -> String {
        let lines = std::mem::take(&mut self.lines);
        self.line = 0;
        self.col = 0;
        lines
            .iter()
            .map(|l| String::from_utf8_lossy(l).into_owned())
            .collect::<Vec<_>>()
            .join("^J")
    }
}

/// Raw mode turns output post-processing off, so every line ends in an explicit CR LF.
fn line(out: &mut impl Write, text: &str) {
    let _ = write!(out, "{text}\r\n");
    let _ = out.flush();
}

/// Sends ESC [ 6 n and reads until the reply's final `R`.
fn ask_cursor(out: &mut impl Write, input: &mut impl Read) {
    if std::env::var_os("FAKE_CLAUDE_SPLIT_DSR").is_some() {
        let _ = out.write_all(b"\x1b[");
        let _ = out.flush();
        std::thread::sleep(std::time::Duration::from_millis(50));
        let _ = out.write_all(b"6n");
    } else {
        let _ = out.write_all(b"\x1b[6n");
    }
    let _ = out.flush();
    let mut b = [0u8; 1];
    while input.read(&mut b).map(|n| n == 1).unwrap_or(false) {
        if b[0] == b'R' {
            return;
        }
    }
}

#[cfg(unix)]
mod raw {
    pub fn enable() {
        // SAFETY: tcgetattr/tcsetattr on fd 0 with a zeroed termios they fill; failure (not a tty) is ignored.
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(0, &mut t) == 0 {
                libc::cfmakeraw(&mut t);
                libc::tcsetattr(0, libc::TCSANOW, &t);
            }
        }
    }

    pub fn drop_input() {
        // SAFETY: discards fd 0's unread input; failure (not a tty) is ignored.
        unsafe {
            libc::tcflush(0, libc::TCIFLUSH);
        }
    }
}

#[cfg(windows)]
mod raw {
    use windows_sys::Win32::System::Console::*;
    pub fn enable() {
        // SAFETY: console mode calls on this process's own std handles; failure (no console) is ignored.
        unsafe {
            let input = GetStdHandle(STD_INPUT_HANDLE);
            let mut mode = 0;
            if GetConsoleMode(input, &mut mode) != 0 {
                let mode = (mode
                    & !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT))
                    | ENABLE_VIRTUAL_TERMINAL_INPUT;
                SetConsoleMode(input, mode);
            }
            let output = GetStdHandle(STD_OUTPUT_HANDLE);
            if GetConsoleMode(output, &mut mode) != 0 {
                SetConsoleMode(
                    output,
                    mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING | DISABLE_NEWLINE_AUTO_RETURN,
                );
            }
        }
    }

    pub fn drop_input() {
        // SAFETY: discards this process's own console input; failure (no console) is ignored.
        unsafe {
            FlushConsoleInputBuffer(GetStdHandle(STD_INPUT_HANDLE));
        }
    }
}
