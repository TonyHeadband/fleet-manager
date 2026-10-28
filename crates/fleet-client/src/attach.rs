//! The terminal side of attaching to an agent: raw mode, keystrokes up the WebSocket, terminal bytes down it.
//! `Ctrl-]` detaches. Temporary until sub-project 3 makes it `fleet attach`.
use std::io::{ErrorKind, Read, Write};
use std::sync::mpsc;
use std::time::Duration;
use tungstenite::{stream::MaybeTlsStream, Message, WebSocket};

pub const DETACH: u8 = 0x1d;

/// Undoes the terminal modes an agent may have turned on: shows the cursor and turns off bracketed paste, focus
/// reports, mouse reporting, the alternate screen and the kitty keyboard protocol.
pub const RESET: &[u8] =
    b"\x1b[?25h\x1b[?2004l\x1b[?1004l\x1b[?1000l\x1b[?1002l\x1b[?1006l\x1b[?1049l\x1b[<u";

/// Windows console mode bits (`ENABLE_VIRTUAL_TERMINAL_INPUT`, `ENABLE_VIRTUAL_TERMINAL_PROCESSING`).
pub const VT_INPUT: u32 = 0x0200;
pub const VT_PROCESSING: u32 = 0x0004;

/// How long a read on the socket waits before the loop goes back to the keyboard.
const READ_WAIT: Duration = Duration::from_millis(20);
/// How often the terminal size is checked: polling works the same on every OS, unlike a resize signal.
const RESIZE_POLL: Duration = Duration::from_millis(250);
const CLOSED: &str = "connection to the daemon closed";

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Detached,
    Exited(u32),
}

/// The bytes to send and whether `Ctrl-]` was among them; nothing after it is sent.
pub fn split_input(buf: &[u8]) -> (Vec<u8>, bool) {
    match buf.iter().position(|b| *b == DETACH) {
        Some(i) => (buf[..i].to_vec(), true),
        None => (buf.to_vec(), false),
    }
}

pub fn ws_url(base: &str, id: &str) -> Result<String, String> {
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!("not an agent id: {id}"));
    }
    let rest = base
        .strip_prefix("http://")
        .ok_or_else(|| format!("the daemon URL must start with http://: {base}"))?;
    Ok(format!(
        "ws://{}/api/agents/{id}/attach",
        rest.trim_end_matches('/')
    ))
}

/// The code in the daemon's `{"exited":CODE}` message, if `text` is one.
pub fn exit_code(text: &str) -> Option<u32> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    u32::try_from(v.get("exited")?.as_u64()?).ok()
}

/// The message in the daemon's `{"error":…}` body, or the body as it is.
fn refusal(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("error")?.as_str().map(String::from))
        .unwrap_or_else(|| body.trim().to_string())
}

/// The console modes while attached, from the ones in force: keys arrive as VT sequences (arrows, Home, F-keys)
/// and the agent's escape sequences are interpreted rather than shown.
pub fn attached_modes(input: u32, output: u32) -> (u32, u32) {
    (input | VT_INPUT, output | VT_PROCESSING)
}

/// Turns UTF-16 console input into UTF-8, holding back a high surrogate whose pair has not been read yet.
#[derive(Default)]
pub struct Utf16ToUtf8 {
    pending: Option<u16>,
}

impl Utf16ToUtf8 {
    pub fn push(&mut self, units: &[u16]) -> Vec<u8> {
        let mut all: Vec<u16> = self.pending.take().into_iter().collect();
        all.extend_from_slice(units);
        if all.last().is_some_and(|u| (0xD800..0xDC00).contains(u)) {
            self.pending = all.pop();
        }
        char::decode_utf16(all)
            .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect::<String>()
            .into_bytes()
    }
}

pub type Socket = WebSocket<MaybeTlsStream<std::net::TcpStream>>;

/// Opens the socket. A refusal before the upgrade comes back as the daemon's own message.
pub fn connect(base: &str, id: &str) -> Result<Socket, String> {
    let url = ws_url(base, id)?;
    match tungstenite::connect(url.as_str()) {
        Ok((ws, _)) => Ok(ws),
        Err(tungstenite::Error::Http(resp)) => {
            let body = resp
                .body()
                .as_deref()
                .map(String::from_utf8_lossy)
                .unwrap_or_default();
            Err(format!("{}: {}", resp.status(), refusal(&body)))
        }
        Err(e) => Err(format!("cannot reach the daemon at {base}: {e}")),
    }
}

enum Input {
    Keys(Vec<u8>),
    Resize(u16, u16),
    /// Stdin is closed, so nothing more can be typed.
    End,
    Failed(String),
}

/// Puts the terminal back however `session` ends, so the shell gets it as it was.
struct RawMode {
    #[cfg(windows)]
    saved: console::Saved,
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
        #[cfg(windows)]
        self.saved.restore_input();
        let mut out = std::io::stdout();
        let _ = out.write_all(RESET).and_then(|()| out.flush());
        // After the reset: a console without VT processing would print it as text.
        #[cfg(windows)]
        self.saved.restore_output();
    }
}

/// Raw mode, plus on Windows the console modes that make keys and output VT.
fn raw_mode() -> Result<RawMode, String> {
    #[cfg(windows)]
    let saved = console::Saved::now();
    crossterm::terminal::enable_raw_mode()
        .map_err(|e| format!("cannot put the terminal in raw mode: {e}"))?;
    #[cfg(windows)]
    console::attach_modes();
    Ok(RawMode {
        #[cfg(windows)]
        saved,
    })
}

fn send_resize(ws: &mut Socket, (cols, rows): (u16, u16)) -> Result<(), String> {
    let msg = serde_json::json!({"resize": {"cols": cols, "rows": rows}}).to_string();
    ws.send(Message::Text(msg.into()))
        .map_err(|_| CLOSED.to_string())
}

/// Reads keystrokes until stdin ends or fails.
fn read_keys(keys: mpsc::Sender<Input>) {
    #[cfg(windows)]
    if console::is_console_input() {
        let mut decode = Utf16ToUtf8::default();
        loop {
            let input = match console::read_input() {
                Ok(units) => Input::Keys(decode.push(&units)),
                Err(e) => Input::Failed(format!("cannot read the keyboard: {e}")),
            };
            let failed = matches!(input, Input::Failed(_));
            if keys.send(input).is_err() || failed {
                return;
            }
        }
    }
    let mut stdin = std::io::stdin();
    let mut buf = [0u8; 4096];
    loop {
        let input = match stdin.read(&mut buf) {
            Ok(0) => Input::End,
            Ok(n) => Input::Keys(buf[..n].to_vec()),
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => Input::Failed(format!("cannot read the keyboard: {e}")),
        };
        let done = matches!(input, Input::End | Input::Failed(_));
        if keys.send(input).is_err() || done {
            return;
        }
    }
}

/// `connect` then `session`, for a caller with nothing to say in between.
pub fn run(base: &str, id: &str) -> Result<Outcome, String> {
    session(connect(base, id)?)
}

/// Takes over this terminal for the agent behind `ws` until `Ctrl-]`, the agent's exit, or a lost connection.
pub fn session(mut ws: Socket) -> Result<Outcome, String> {
    if let MaybeTlsStream::Plain(s) = ws.get_mut() {
        s.set_read_timeout(Some(READ_WAIT))
            .map_err(|e| format!("cannot set up the connection: {e}"))?;
    }
    let _raw = raw_mode()?;

    let mut size = crossterm::terminal::size().ok();
    if let Some(s) = size {
        send_resize(&mut ws, s)?;
    }
    let (tx, rx) = mpsc::channel();
    let keys = tx.clone();
    std::thread::spawn(move || read_keys(keys));
    std::thread::spawn(move || loop {
        std::thread::sleep(RESIZE_POLL);
        let now = crossterm::terminal::size().ok();
        if now != size {
            size = now;
            if let Some((cols, rows)) = now {
                if tx.send(Input::Resize(cols, rows)).is_err() {
                    return;
                }
            }
        }
    });

    let mut out = std::io::stdout();
    loop {
        while let Ok(input) = rx.try_recv() {
            match input {
                Input::Keys(buf) => {
                    let (send, detach) = split_input(&buf);
                    if !send.is_empty() {
                        ws.send(Message::Binary(send.into()))
                            .map_err(|_| CLOSED.to_string())?;
                    }
                    if detach {
                        let _ = ws.close(None);
                        let _ = ws.flush();
                        return Ok(Outcome::Detached);
                    }
                }
                Input::Resize(cols, rows) => send_resize(&mut ws, (cols, rows))?,
                Input::End => return Ok(Outcome::Detached),
                Input::Failed(e) => return Err(e),
            }
        }
        match ws.read() {
            Ok(Message::Binary(b)) => {
                out.write_all(&b)
                    .and_then(|()| out.flush())
                    .map_err(|e| format!("cannot write to the terminal: {e}"))?;
            }
            Ok(Message::Text(t)) => {
                if let Some(code) = exit_code(&t) {
                    return Ok(Outcome::Exited(code));
                }
            }
            Ok(Message::Close(_)) => return Err(CLOSED.into()),
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(_) => return Err(CLOSED.into()),
        }
    }
}

/// The Windows console: crossterm's raw mode leaves keys as key events, which std's `Stdin` drops unless they
/// carry a character (arrows, Home, F-keys), and std's `Stdin` takes a lone `Ctrl-Z` for the end of input. So
/// keys are read as VT sequences straight from the console. Without a console (a pipe), none of this applies.
#[cfg(windows)]
mod console {
    use windows_sys::Win32::System::Console::{
        GetConsoleMode, GetStdHandle, ReadConsoleW, SetConsoleMode, STD_HANDLE, STD_INPUT_HANDLE,
        STD_OUTPUT_HANDLE,
    };

    fn mode(which: STD_HANDLE) -> Option<u32> {
        let mut m = 0;
        // SAFETY: GetStdHandle has no preconditions and GetConsoleMode only writes `m`.
        (unsafe { GetConsoleMode(GetStdHandle(which), &mut m) } != 0).then_some(m)
    }

    fn set_mode(which: STD_HANDLE, m: u32) {
        // SAFETY: plain calls on this process's standard handle.
        unsafe { SetConsoleMode(GetStdHandle(which), m) };
    }

    /// The modes from before raw mode, put back exactly on the way out.
    pub struct Saved {
        input: Option<u32>,
        output: Option<u32>,
    }

    impl Saved {
        pub fn now() -> Saved {
            Saved {
                input: mode(STD_INPUT_HANDLE),
                output: mode(STD_OUTPUT_HANDLE),
            }
        }

        pub fn restore_input(&self) {
            if let Some(m) = self.input {
                set_mode(STD_INPUT_HANDLE, m);
            }
        }

        pub fn restore_output(&self) {
            if let Some(m) = self.output {
                set_mode(STD_OUTPUT_HANDLE, m);
            }
        }
    }

    /// Adds VT input to raw mode's input mode and VT processing to the output mode, where they are consoles.
    pub fn attach_modes() {
        let (input, output) = (mode(STD_INPUT_HANDLE), mode(STD_OUTPUT_HANDLE));
        let (vt_in, vt_out) = super::attached_modes(input.unwrap_or(0), output.unwrap_or(0));
        if input.is_some() {
            set_mode(STD_INPUT_HANDLE, vt_in);
        }
        if output.is_some() {
            set_mode(STD_OUTPUT_HANDLE, vt_out);
        }
    }

    pub fn is_console_input() -> bool {
        mode(STD_INPUT_HANDLE).is_some()
    }

    /// Blocks for the next keys, as UTF-16.
    pub fn read_input() -> std::io::Result<Vec<u16>> {
        let mut buf = [0u16; 1024];
        let mut n = 0u32;
        // SAFETY: `buf` holds `buf.len()` units and ReadConsoleW writes at most that many, reporting how many in `n`.
        let ok = unsafe {
            ReadConsoleW(
                GetStdHandle(STD_INPUT_HANDLE),
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
                &mut n,
                std::ptr::null(),
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(buf[..n as usize].to_vec())
    }

    #[cfg(test)]
    mod tests {
        use windows_sys::Win32::System::Console::{
            ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
        };

        #[test]
        fn the_mode_bits_are_windows_own() {
            assert_eq!(super::super::VT_INPUT, ENABLE_VIRTUAL_TERMINAL_INPUT);
            assert_eq!(
                super::super::VT_PROCESSING,
                ENABLE_VIRTUAL_TERMINAL_PROCESSING
            );
        }
    }
}
