//! The terminal side of attaching to an agent: raw mode, keystrokes up the WebSocket, terminal bytes down it.
//! `Ctrl-]` detaches. Temporary until sub-project 3 makes it `fleet attach`.
use std::io::{ErrorKind, Read, Write};
use std::sync::mpsc;
use std::time::Duration;
use tungstenite::{stream::MaybeTlsStream, Message, WebSocket};

pub const DETACH: u8 = 0x1d;

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
            Err(format!("{}: {}", resp.status(), body.trim()))
        }
        Err(e) => Err(format!("cannot reach the daemon at {base}: {e}")),
    }
}

enum Input {
    Keys(Vec<u8>),
    Resize(u16, u16),
    /// Stdin is closed, so nothing more can be typed.
    End,
}

/// Leaves raw mode however `run` ends, so the shell gets its terminal back.
struct RawMode;

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

fn send_resize(ws: &mut Socket, (cols, rows): (u16, u16)) -> Result<(), String> {
    let msg = serde_json::json!({"resize": {"cols": cols, "rows": rows}}).to_string();
    ws.send(Message::Text(msg.into()))
        .map_err(|_| CLOSED.to_string())
}

/// Takes over this terminal for agent `id` until `Ctrl-]`, the agent's exit, or a lost connection.
pub fn run(base: &str, id: &str) -> Result<Outcome, String> {
    let mut ws = connect(base, id)?;
    if let MaybeTlsStream::Plain(s) = ws.get_mut() {
        s.set_read_timeout(Some(READ_WAIT))
            .map_err(|e| format!("cannot set up the connection: {e}"))?;
    }
    crossterm::terminal::enable_raw_mode()
        .map_err(|e| format!("cannot put the terminal in raw mode: {e}"))?;
    let _raw = RawMode;
    // A Windows console shows the agent's escape sequences as text until it is told to interpret them.
    #[cfg(windows)]
    crossterm::ansi_support::supports_ansi();

    let mut size = crossterm::terminal::size().ok();
    if let Some(s) = size {
        send_resize(&mut ws, s)?;
    }
    let (tx, rx) = mpsc::channel();
    let keys = tx.clone();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut buf = [0u8; 4096];
        loop {
            let input = match stdin.read(&mut buf) {
                Ok(0) | Err(_) => Input::End,
                Ok(n) => Input::Keys(buf[..n].to_vec()),
            };
            let end = matches!(input, Input::End);
            if keys.send(input).is_err() || end {
                return;
            }
        }
    });
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
