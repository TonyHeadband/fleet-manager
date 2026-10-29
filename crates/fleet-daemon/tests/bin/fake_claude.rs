//! Test double for the claude CLI. Reports how it was started, then echoes keystrokes one at a time in `cat -v`
//! style (Escape as `^[`, Ctrl-U as `^U`) so tests can see exactly what the daemon typed. Like ConPTY at startup it
//! first asks where the cursor is and prints nothing until the terminal answers.
use std::io::{Read, Write};

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
    // Tests wait for this line: input sent before raw mode was on could have been flushed.
    line(&mut out, "ready");
    if let Ok(code) = std::env::var("FAKE_CLAUDE_EXIT") {
        std::process::exit(code.parse().unwrap_or(1));
    }
    if let Ok(n) = std::env::var("FAKE_CLAUDE_BURST") {
        for i in 1..=n.parse::<u32>().unwrap_or(0) {
            line(&mut out, &format!("burst {i}"));
        }
    }
    let mut b = [0u8; 1];
    while input.read(&mut b).map(|n| n == 1).unwrap_or(false) {
        let shown = match b[0] {
            b'\r' | b'\n' => "\r\n".to_string(),
            0x7f => "^?".to_string(),
            c if c < 0x20 => format!("^{}", (c + 0x40) as char),
            c => (c as char).to_string(),
        };
        let _ = out.write_all(shown.as_bytes());
        let _ = out.flush();
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
}
