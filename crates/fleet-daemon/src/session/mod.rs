//! Agent terminals the daemon owns: one pseudo-terminal per agent, on every OS (ConPTY on Windows). A reader
//! thread keeps a vt100 screen of it, fans its output out to attached terminals, and answers the cursor-position
//! query ConPTY sends at startup: nothing is drawn until that query is answered, and usually no terminal is
//! attached to answer it.
pub mod resolve;
mod tree;

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::num::NonZeroU16;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::broadcast;

pub const DEFAULT_SIZE: Size = Size {
    cols: 120,
    rows: 40,
};
/// Output chunks a watcher may fall behind by before it is resynced with a snapshot.
const BACKLOG: usize = 1024;
const CURSOR_QUERY: &[u8] = b"\x1b[6n";

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Enter,
    Escape,
    CtrlU,
}

impl Key {
    fn bytes(self) -> &'static [u8] {
        match self {
            Key::Enter => b"\r",
            Key::Escape => b"\x1b",
            // Clears the input line; a double Escape does not.
            Key::CtrlU => b"\x15",
        }
    }
}

#[derive(Clone, Debug)]
pub enum Output {
    Bytes(Arc<[u8]>),
    Exited(u32),
}

pub enum Recv {
    Bytes(Arc<[u8]>),
    /// The terminal fell behind and missed output: redraw from this.
    Lagged(Vec<u8>),
    Exited(u32),
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("no session named {0}")]
    Missing(String),
    #[error("a session named {0} already exists")]
    Exists(String),
    #[error("could not open a terminal: {0}")]
    Pty(String),
    #[error("could not start {program}: {error}")]
    Spawn { program: String, error: String },
    #[error("cannot type into session {name}: {source}")]
    Write {
        name: String,
        source: std::io::Error,
    },
}

pub struct Spec<'a> {
    pub argv: &'a [String],
    pub cwd: &'a Path,
    pub env: &'a [(&'a str, &'a str)],
    /// Variables the program must not inherit from the daemon (ambient tokens).
    pub remove_env: &'a [&'a str],
}

struct Session {
    /// Taken (and so closed) once the child exits: on Windows the reader sees EOF only when the pseudo-console closes.
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    screen: Arc<Mutex<vt100::Parser>>,
    tx: broadcast::Sender<Output>,
    tree: tree::Tree,
    /// Attached terminals by id with their size, the most recent resize last.
    sizes: Mutex<Vec<(u64, Size)>>,
}

type Map = Mutex<HashMap<String, Arc<Session>>>;

#[derive(Clone, Default)]
pub struct SessionHost {
    sessions: Arc<Map>,
    next_id: Arc<AtomicU64>,
}

fn nz(n: u16) -> NonZeroU16 {
    NonZeroU16::new(n.max(1)).expect("max(1) is never zero")
}

fn pty_size(s: Size) -> PtySize {
    PtySize {
        rows: s.rows.max(1),
        cols: s.cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}

impl SessionHost {
    pub fn new() -> SessionHost {
        SessionHost::default()
    }

    fn get(&self, name: &str) -> Result<Arc<Session>, SessionError> {
        self.sessions
            .lock()
            .expect("sessions poisoned")
            .get(name)
            .cloned()
            .ok_or_else(|| SessionError::Missing(name.to_string()))
    }

    pub fn start(&self, name: &str, spec: Spec) -> Result<(), SessionError> {
        let mut sessions = self.sessions.lock().expect("sessions poisoned");
        if sessions.contains_key(name) {
            return Err(SessionError::Exists(name.to_string()));
        }
        let Some((program, args)) = spec.argv.split_first() else {
            return Err(SessionError::Spawn {
                program: String::new(),
                error: "empty command line".into(),
            });
        };
        let program = program.clone();
        let pair = native_pty_system()
            .openpty(pty_size(DEFAULT_SIZE))
            .map_err(|e| SessionError::Pty(e.to_string()))?;
        // Taken before the spawn so a failure here cannot leave a running child behind.
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| SessionError::Pty(e.to_string()))?;
        let writer: Arc<Mutex<Box<dyn Write + Send>>> = Arc::new(Mutex::new(
            pair.master
                .take_writer()
                .map_err(|e| SessionError::Pty(e.to_string()))?,
        ));
        let mut cmd = CommandBuilder::new(&program);
        cmd.args(args);
        cmd.cwd(spec.cwd);
        for k in spec.remove_env {
            cmd.env_remove(k);
        }
        for (k, v) in spec.env {
            cmd.env(k, v);
        }
        let spawn_err = |e: String| SessionError::Spawn {
            program: program.clone(),
            error: e,
        };
        let mut child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| spawn_err(e.to_string()))?;
        drop(pair.slave);
        let pid = child
            .process_id()
            .ok_or_else(|| spawn_err("exited at once".into()))?;
        let tree = match tree::Tree::adopt(pid) {
            Ok(t) => t,
            Err(e) => {
                let _ = child.kill();
                return Err(spawn_err(e.to_string()));
            }
        };
        let screen = Arc::new(Mutex::new(vt100::Parser::new(
            nz(DEFAULT_SIZE.rows),
            nz(DEFAULT_SIZE.cols),
            0,
        )));
        let (tx, _) = broadcast::channel(BACKLOG);
        let session = Arc::new(Session {
            master: Mutex::new(Some(pair.master)),
            writer: writer.clone(),
            screen: screen.clone(),
            tx: tx.clone(),
            tree,
            sizes: Mutex::new(Vec::new()),
        });
        sessions.insert(name.to_string(), session.clone());
        drop(sessions);

        let pump = std::thread::spawn(move || pump(reader, &screen, &writer, &tx));
        let map = Arc::downgrade(&self.sessions);
        let name = name.to_string();
        std::thread::spawn(move || {
            let code = child.wait().map(|s| s.exit_code()).unwrap_or(1);
            // Closing the pseudo-console ends the reader on Windows; on Unix it reads to EOF on its own.
            session.master.lock().expect("master poisoned").take();
            let _ = pump.join();
            remove_if_same(&map, &name, &session);
            let _ = session.tx.send(Output::Exited(code));
        });
        Ok(())
    }

    pub fn write(&self, name: &str, bytes: &[u8]) -> Result<(), SessionError> {
        let s = self.get(name)?;
        let mut w = s.writer.lock().expect("writer poisoned");
        w.write_all(bytes)
            .and_then(|()| w.flush())
            .map_err(|source| SessionError::Write {
                name: name.to_string(),
                source,
            })
    }

    pub fn write_text(&self, name: &str, text: &str) -> Result<(), SessionError> {
        self.write(name, text.as_bytes())
    }

    pub fn write_key(&self, name: &str, key: Key) -> Result<(), SessionError> {
        self.write(name, key.bytes())
    }

    pub fn screen_text(&self, name: &str) -> Result<String, SessionError> {
        Ok(self
            .get(name)?
            .screen
            .lock()
            .expect("screen poisoned")
            .screen()
            .contents())
    }

    pub fn size(&self, name: &str) -> Result<Size, SessionError> {
        let s = self.get(name)?;
        let (rows, cols) = s.screen.lock().expect("screen poisoned").screen().size();
        Ok(Size {
            cols: cols.get(),
            rows: rows.get(),
        })
    }

    pub fn alive(&self, name: &str) -> bool {
        self.sessions
            .lock()
            .expect("sessions poisoned")
            .contains_key(name)
    }

    pub fn list(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .sessions
            .lock()
            .expect("sessions poisoned")
            .keys()
            .cloned()
            .collect();
        names.sort();
        names
    }

    /// Kills the session's whole process tree. The name is free at once; the exit is still reported to watchers.
    pub fn kill(&self, name: &str) -> Result<(), SessionError> {
        let s = self
            .sessions
            .lock()
            .expect("sessions poisoned")
            .remove(name)
            .ok_or_else(|| SessionError::Missing(name.to_string()))?;
        s.tree.kill();
        Ok(())
    }

    pub fn shutdown_all(&self) {
        for name in self.list() {
            let _ = self.kill(&name);
        }
    }

    pub fn watch(&self, name: &str) -> Result<broadcast::Receiver<Output>, SessionError> {
        Ok(self.get(name)?.tx.subscribe())
    }

    /// A terminal attaching at `size`, which becomes the session's size until another terminal resizes.
    pub fn attach(&self, name: &str, size: Size) -> Result<Attachment, SessionError> {
        let session = self.get(name)?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        // Subscribed under the screen lock the reader takes for each chunk, so no output falls between the
        // snapshot a terminal starts from and the stream it continues with.
        let rx = {
            let _screen = session.screen.lock().expect("screen poisoned");
            session.tx.subscribe()
        };
        session
            .sizes
            .lock()
            .expect("sizes poisoned")
            .push((id, size));
        apply_size(&session);
        Ok(Attachment { session, id, rx })
    }
}

pub struct Attachment {
    session: Arc<Session>,
    id: u64,
    rx: broadcast::Receiver<Output>,
}

impl Attachment {
    pub fn snapshot(&self) -> Vec<u8> {
        self.session
            .screen
            .lock()
            .expect("screen poisoned")
            .screen()
            .state_formatted()
            .into_bytes()
    }

    pub async fn recv(&mut self) -> Option<Recv> {
        match self.rx.recv().await {
            Ok(Output::Bytes(b)) => Some(Recv::Bytes(b)),
            Ok(Output::Exited(c)) => Some(Recv::Exited(c)),
            Err(broadcast::error::RecvError::Lagged(_)) => {
                // Resubscribe first: output after this point is streamed, output before it is in the snapshot.
                self.rx = self.rx.resubscribe();
                Some(Recv::Lagged(self.snapshot()))
            }
            Err(broadcast::error::RecvError::Closed) => None,
        }
    }

    pub fn resize(&self, size: Size) {
        {
            let mut sizes = self.session.sizes.lock().expect("sizes poisoned");
            sizes.retain(|(id, _)| *id != self.id);
            sizes.push((self.id, size));
        }
        apply_size(&self.session);
    }
}

impl Drop for Attachment {
    fn drop(&mut self) {
        self.session
            .sizes
            .lock()
            .expect("sizes poisoned")
            .retain(|(id, _)| *id != self.id);
        apply_size(&self.session);
    }
}

/// The most recent terminal's size, or the default with none attached. An unchanged size is left alone: ConPTY
/// answers every resize with a full repaint, which would reach attached terminals as noise.
fn apply_size(s: &Session) {
    // Held throughout so concurrent resizes apply in order.
    let sizes = s.sizes.lock().expect("sizes poisoned");
    let size = sizes.last().map_or(DEFAULT_SIZE, |(_, z)| *z);
    let (rows, cols) = s.screen.lock().expect("screen poisoned").screen().size();
    if (rows.get(), cols.get()) == (size.rows.max(1), size.cols.max(1)) {
        return;
    }
    if let Some(m) = s.master.lock().expect("master poisoned").as_ref() {
        let _ = m.resize(pty_size(size));
    }
    s.screen
        .lock()
        .expect("screen poisoned")
        .set_size(nz(size.rows), nz(size.cols));
}

fn remove_if_same(map: &Weak<Map>, name: &str, s: &Arc<Session>) {
    let Some(map) = map.upgrade() else { return };
    let mut m = map.lock().expect("sessions poisoned");
    if m.get(name).is_some_and(|x| Arc::ptr_eq(x, s)) {
        m.remove(name);
    }
}

/// Reads the PTY until it closes: feeds the screen, fans the bytes out, answers cursor queries.
fn pump(
    mut reader: Box<dyn Read + Send>,
    screen: &Mutex<vt100::Parser>,
    writer: &Mutex<Box<dyn Write + Send>>,
    tx: &broadcast::Sender<Output>,
) {
    let mut buf = [0u8; 8192];
    // The last bytes of the previous read, so a query split across two reads is still seen.
    let mut tail: Vec<u8> = Vec::new();
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        let chunk = &buf[..n];
        let mut p = screen.lock().expect("screen poisoned");
        p.process(chunk);
        tail.extend_from_slice(chunk);
        if tail.windows(CURSOR_QUERY.len()).any(|w| w == CURSOR_QUERY) {
            let (r, c) = p.screen().cursor_position();
            let mut w = writer.lock().expect("writer poisoned");
            let _ = w.write_all(format!("\x1b[{};{}R", r + 1, c + 1).as_bytes());
            let _ = w.flush();
            tail.clear();
        }
        let keep = tail.len().saturating_sub(CURSOR_QUERY.len() - 1);
        tail.drain(..keep);
        let _ = tx.send(Output::Bytes(chunk.into()));
        drop(p);
    }
}
