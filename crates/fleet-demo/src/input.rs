//! Keystrokes from the tmux pane: the daemon answers with literal text plus Enter, and pauses with Escape.
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    Line(String),
    Escape,
    Eof,
}

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// Set by SIGHUP/SIGTERM so the player can stop at its next tick instead of finishing a scripted delay.
pub fn shutdown_requested() -> bool {
    SHUTDOWN.load(Ordering::SeqCst)
}

struct TtyGuard(Option<libc::termios>);

impl TtyGuard {
    /// No-op when stdin is not a tty, so tests and pipes work unchanged.
    fn raw_stdin() -> Self {
        // SAFETY: termios is plain data; tcgetattr fills it or fails.
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(0, &mut t) != 0 {
                return Self(None);
            }
            let saved = t;
            t.c_lflag &= !(libc::ICANON | libc::ECHO);
            t.c_cc[libc::VMIN] = 1;
            t.c_cc[libc::VTIME] = 0;
            libc::tcsetattr(0, libc::TCSANOW, &t);
            Self(Some(saved))
        }
    }
}

impl Drop for TtyGuard {
    fn drop(&mut self) {
        if let Some(t) = &self.0 {
            // SAFETY: restores the attributes read in raw_stdin.
            unsafe { libc::tcsetattr(0, libc::TCSANOW, t) };
        }
    }
}

/// Turns raw bytes into inputs. A read that is exactly one ESC is the Escape key; a longer read that starts
/// with ESC is a terminal sequence (arrows, bracketed paste) and is dropped.
struct Decoder {
    buf: Vec<u8>,
    after_cr: bool,
}

impl Decoder {
    fn new() -> Self {
        Self {
            buf: Vec::new(),
            after_cr: false,
        }
    }

    fn feed(&mut self, chunk: &[u8], out: &mut Vec<Input>) {
        if chunk == [0x1b] {
            out.push(Input::Escape);
            return;
        }
        if chunk.first() == Some(&0x1b) {
            return;
        }
        for &b in chunk {
            match b {
                b'\r' | b'\n' => {
                    // CRLF from a terminal that does not translate is one Enter, not two.
                    if b == b'\n' && self.after_cr {
                        self.after_cr = false;
                        continue;
                    }
                    self.after_cr = b == b'\r';
                    let text = String::from_utf8_lossy(&self.buf).into_owned();
                    self.buf.clear();
                    out.push(Input::Line(text));
                }
                _ => {
                    self.after_cr = false;
                    self.buf.push(b);
                }
            }
        }
    }
}

/// Reads stdin on a thread. SIGHUP/SIGTERM are blocked process-wide and consumed by a second thread that
/// sends `Eof`, so a blocked `recv` wakes up and the player can post SessionEnd.
pub fn spawn_stdin_reader() -> Receiver<Input> {
    let (tx, rx) = mpsc::channel();
    watch_signals(tx.clone());
    thread::spawn(move || {
        let _tty = TtyGuard::raw_stdin();
        let mut dec = Decoder::new();
        let mut stdin = std::io::stdin().lock();
        let mut chunk = [0u8; 256];
        loop {
            let n = match stdin.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            };
            let mut found = Vec::new();
            dec.feed(&chunk[..n], &mut found);
            for i in found {
                if tx.send(i).is_err() {
                    return;
                }
            }
        }
        let _ = tx.send(Input::Eof);
    });
    rx
}

fn watch_signals(tx: Sender<Input>) {
    // SAFETY: the mask is set before any thread is spawned, so every thread inherits it and the
    // sigwait thread is the only consumer of these signals.
    let set = unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGHUP);
        libc::sigaddset(&mut set, libc::SIGTERM);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        set
    };
    thread::spawn(move || {
        let mut sig = 0;
        // SAFETY: `set` is initialised above and outlives the call.
        unsafe { libc::sigwait(&set, &mut sig) };
        SHUTDOWN.store(true, Ordering::SeqCst);
        let _ = tx.send(Input::Eof);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(chunks: &[&[u8]]) -> Vec<Input> {
        let mut d = Decoder::new();
        let mut out = Vec::new();
        for c in chunks {
            d.feed(c, &mut out);
        }
        out
    }

    #[test]
    fn lone_escape_and_split_line() {
        assert_eq!(
            decode(&[b"\x1b", b"con", b"tinue\r"]),
            [Input::Escape, Input::Line("continue".into())]
        );
    }

    #[test]
    fn crlf_is_one_enter_and_sequences_are_dropped() {
        assert_eq!(
            decode(&[b"a\r\n", b"\x1b[A", b"b\n"]),
            [Input::Line("a".into()), Input::Line("b".into())]
        );
    }
}
