//! Settings → Log: the app's and the daemon's log lines in one list, from `gui.log` and `daemon.log` in the state
//! directory plus what this window logged since it started.
use crate::theme::{mono, size, tokens};
use crate::widgets::{chip, toggle};
use eframe::egui::{self, text::LayoutJob, Color32, RichText, ScrollArea, TextEdit, TextFormat, Ui};
use fleet_log::{render, Entry, Level};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

/// The most read from the end of a log file; older lines stay on disk.
pub const TAIL: u64 = 1024 * 1024;

/// How often the daemon's file is checked for new lines while the tab is on screen.
const POLL: Duration = Duration::from_secs(1);

/// The last `max` bytes of `path`, cut at a line start, and the file's length when it was read. A file that fits
/// comes back whole.
pub fn tail(path: &Path, max: u64) -> io::Result<(String, u64)> {
    let (bytes, end) = tail_bytes(path, max)?;
    Ok((String::from_utf8_lossy(&bytes).into_owned(), end))
}

// Bytes, not text: offsets must stay exact when a line holds invalid UTF-8.
fn tail_bytes(path: &Path, max: u64) -> io::Result<(Vec<u8>, u64)> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let mut buf = Vec::new();
    if len <= max {
        file.take(len).read_to_end(&mut buf)?;
        return Ok((buf, len));
    }
    // Starts one byte early: when that byte is a newline, the cut already is a line start.
    let from = len - max - 1;
    file.seek(SeekFrom::Start(from))?;
    file.take(max + 1).read_to_end(&mut buf)?;
    let end = from + buf.len() as u64;
    let skip = if buf.first() == Some(&b'\n') { 1 } else { buf.iter().position(|&b| b == b'\n').map_or(buf.len(), |i| i + 1) };
    buf.drain(..skip);
    Ok((buf, end))
}

/// Lines read from a file and the offset just after the last whole one. A line still being written is left for the
/// next read, or it would show up twice, as two fragments.
struct Chunk {
    entries: Vec<Entry>,
    end: u64,
}

fn whole_lines(bytes: &[u8], end: u64) -> Chunk {
    let keep = bytes.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    let entries = fleet_log::parse_file(&String::from_utf8_lossy(&bytes[..keep]));
    Chunk { entries, end: end - (bytes.len() - keep) as u64 }
}

struct Update {
    /// The file shrank (truncated or rotated): `chunk` replaces what was shown.
    reset: bool,
    chunk: Chunk,
}

fn read_new(path: &Path, offset: u64) -> io::Result<Update> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let reset = len < offset;
    let start = if reset { 0 } else { offset };
    if len - start > TAIL {
        let (bytes, end) = tail_bytes(path, TAIL)?;
        return Ok(Update { reset: true, chunk: whole_lines(&bytes, end) });
    }
    let mut bytes = Vec::new();
    file.seek(SeekFrom::Start(start))?;
    file.take(len - start).read_to_end(&mut bytes)?;
    Ok(Update { reset, chunk: whole_lines(&bytes, start + bytes.len() as u64) })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    App,
    Daemon,
    Both,
}

enum Msg {
    Loaded { app: io::Result<Chunk>, daemon: io::Result<Chunk> },
    Polled(io::Result<Update>),
}

enum FileState {
    Pending,
    /// The window does not know where the state directory is.
    Unknown,
    Missing,
    Failed(String),
    Read,
}

struct FileLog {
    path: Option<PathBuf>,
    entries: Vec<Entry>,
    offset: u64,
    state: FileState,
}

impl FileLog {
    fn new() -> FileLog {
        FileLog { path: None, entries: Vec::new(), offset: 0, state: FileState::Pending }
    }

    fn load(&mut self, result: io::Result<Chunk>) {
        match result {
            Ok(chunk) => {
                self.entries = chunk.entries;
                self.offset = chunk.end;
                self.state = FileState::Read;
            }
            Err(e) => self.fail(e),
        }
    }

    fn fail(&mut self, e: io::Error) {
        self.entries.clear();
        self.offset = 0;
        self.state = if e.kind() == io::ErrorKind::NotFound { FileState::Missing } else { FileState::Failed(e.to_string()) };
    }

    fn notice(&self, what: &str) -> Option<String> {
        let path = self.path.as_ref().map(|p| p.display().to_string());
        match (&self.state, path) {
            (FileState::Missing, Some(p)) => Some(format!("No {what} log at {p}")),
            (FileState::Failed(e), Some(p)) => Some(format!("Cannot read the {what} log at {p}: {e}")),
            (FileState::Unknown, _) => Some(format!("No {what} log: the state directory is unknown")),
            _ => None,
        }
    }
}

#[derive(PartialEq)]
struct Key {
    version: u64,
    pushed: u64,
    since: Option<i64>,
    source: Source,
    levels: [bool; 4],
    search: String,
}

/// One line of the list: an index into the app's lines (file then live) or into the daemon's.
#[derive(Clone, Copy)]
struct Row {
    daemon: bool,
    i: usize,
}

struct Built {
    key: Key,
    rows: Vec<Row>,
    /// File lines of the app that are listed; the live ones follow them.
    app_file: usize,
}

pub struct LogView {
    source: Source,
    search: String,
    /// Error, warning, info, debug (trace goes with debug).
    levels: [bool; 4],
    app: FileLog,
    daemon: FileLog,
    started: bool,
    loaded: bool,
    polling: bool,
    last_poll: Instant,
    /// Counts changes to the files' lines, so the list is rebuilt only when its inputs changed.
    version: u64,
    built: Option<Built>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl Default for LogView {
    fn default() -> LogView {
        LogView::new()
    }
}

impl LogView {
    pub fn new() -> LogView {
        let (tx, rx) = channel();
        LogView {
            source: Source::Both,
            search: String::new(),
            levels: [true, true, true, false],
            app: FileLog::new(),
            daemon: FileLog::new(),
            started: false,
            loaded: false,
            polling: false,
            last_poll: Instant::now(),
            version: 0,
            built: None,
            tx,
            rx,
        }
    }

    pub fn source(&self) -> Source {
        self.source
    }

    /// Reads both files once, on a background thread, the first time the tab is shown.
    fn start(&mut self, ctx: &egui::Context, state_dir: Option<&Path>) {
        if std::mem::replace(&mut self.started, true) {
            return;
        }
        let Some(dir) = state_dir else {
            self.app.state = FileState::Unknown;
            self.daemon.state = FileState::Unknown;
            self.loaded = true;
            return;
        };
        let (app, daemon) = (dir.join("gui.log"), dir.join("daemon.log"));
        self.app.path = Some(app.clone());
        self.daemon.path = Some(daemon.clone());
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let read = |p: &Path| tail_bytes(p, TAIL).map(|(bytes, end)| whole_lines(&bytes, end));
            let _ = tx.send(Msg::Loaded { app: read(&app), daemon: read(&daemon) });
            ctx.request_repaint();
        });
    }

    fn receive(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            self.version += 1;
            match msg {
                Msg::Loaded { app, daemon } => {
                    self.app.load(app);
                    self.daemon.load(daemon);
                    self.loaded = true;
                    self.last_poll = Instant::now();
                }
                Msg::Polled(result) => {
                    self.polling = false;
                    self.last_poll = Instant::now();
                    match result {
                        Ok(update) => self.daemon.apply(update),
                        Err(e) => self.daemon.fail(e),
                    }
                }
            }
        }
    }

    /// Checks the daemon's file for new lines at most once a second. Only called while the tab is on screen, so a
    /// hidden tab does no file IO.
    fn poll(&mut self, ctx: &egui::Context) {
        let Some(path) = self.daemon.path.clone() else { return };
        if !self.loaded || self.polling || self.last_poll.elapsed() < POLL {
            return;
        }
        self.polling = true;
        let (offset, tx, ctx) = (self.daemon.offset, self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Polled(read_new(&path, offset)));
            ctx.request_repaint();
        });
    }

    pub fn show(&mut self, ui: &mut Ui, pane: LogFeed<'_>) {
        let ctx = ui.ctx().clone();
        self.start(&ctx, pane.state_dir);
        self.receive();
        self.poll(&ctx);
        // The only repaint this tab asks for, and only while it is drawn.
        ctx.request_repaint_after(POLL);

        self.controls(ui);
        ui.add_space(8.0);
        self.rebuild(&pane);
        let p = tokens(ui.visuals().dark_mode);
        let notices = [(self.source != Source::Daemon).then(|| self.app.notice("app")).flatten(), (self.source != Source::App).then(|| self.daemon.notice("daemon")).flatten()];
        for notice in notices.into_iter().flatten() {
            ui.label(RichText::new(notice).size(size::SMALL).color(p.text_low));
        }
        let Some(built) = &self.built else { return };
        let app_at = |i: usize| if i < built.app_file { &self.app.entries[i] } else { &pane.live[i - built.app_file] };
        let font = mono(size::CODE);
        let height = ui.fonts_mut(|f| f.row_height(&font));
        ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show_rows(ui, height, built.rows.len(), |ui, range| {
            for row in &built.rows[range] {
                let entry = if row.daemon { &self.daemon.entries[row.i] } else { app_at(row.i) };
                ui.add(egui::Label::new(line(entry, &font, ui.visuals().dark_mode)).truncate());
            }
        });
    }

    fn controls(&mut self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            for (source, label) in [(Source::App, "App"), (Source::Daemon, "Daemon"), (Source::Both, "Both")] {
                if chip(ui, label, self.source == source).clicked() {
                    self.source = source;
                }
            }
            ui.add_space(8.0);
            for (on, label) in self.levels.iter_mut().zip(["error", "warning", "info", "debug"]) {
                if toggle(ui, label, *on).clicked() {
                    *on = !*on;
                }
            }
            ui.add_space(8.0);
            ui.add(TextEdit::singleline(&mut self.search).hint_text("Search").desired_width(220.0));
        });
    }

    /// Rebuilds the merged, filtered list when something it depends on changed; otherwise a frame costs nothing.
    fn rebuild(&mut self, pane: &LogFeed<'_>) {
        let key = Key { version: self.version, pushed: pane.live_pushed, since: pane.live_since, source: self.source, levels: self.levels, search: self.search.clone() };
        if self.built.as_ref().is_some_and(|b| b.key == key) {
            return;
        }
        let app_file = match pane.live_since {
            Some(t) => self.app.entries.iter().take_while(|e| e.time < t).count(),
            None => self.app.entries.len(),
        };
        let app_len = app_file + pane.live.len();
        let needle = self.search.to_lowercase();
        let levels = self.levels;
        let keep = |e: &Entry| {
            let shown = match e.level {
                None => true,
                Some(Level::Error) => levels[0],
                Some(Level::Warn) => levels[1],
                Some(Level::Info) => levels[2],
                Some(Level::Debug | Level::Trace) => levels[3],
            };
            shown && (needle.is_empty() || render(e).to_lowercase().contains(&needle))
        };
        let app_at = |i: usize| if i < app_file { &self.app.entries[i] } else { &pane.live[i - app_file] };
        let daemon = &self.daemon.entries;
        let (app_wanted, daemon_wanted) = (self.source != Source::Daemon, self.source != Source::App);
        let (mut a, mut d) = (0, 0);
        let mut rows = Vec::new();
        loop {
            while a < app_len && !(app_wanted && keep(app_at(a))) {
                a += 1;
            }
            while d < daemon.len() && !(daemon_wanted && keep(&daemon[d])) {
                d += 1;
            }
            // Ties list the app's line first.
            match (a < app_len, d < daemon.len()) {
                (false, false) => break,
                (true, false) => {
                    rows.push(Row { daemon: false, i: a });
                    a += 1;
                }
                (false, true) => {
                    rows.push(Row { daemon: true, i: d });
                    d += 1;
                }
                (true, true) if app_at(a).time <= daemon[d].time => {
                    rows.push(Row { daemon: false, i: a });
                    a += 1;
                }
                (true, true) => {
                    rows.push(Row { daemon: true, i: d });
                    d += 1;
                }
            }
        }
        self.built = Some(Built { key, rows, app_file });
    }
}

impl FileLog {
    fn apply(&mut self, update: Update) {
        let Update { reset, chunk } = update;
        self.offset = chunk.end;
        self.state = FileState::Read;
        let mut entries = chunk.entries;
        if reset {
            self.entries = entries;
            return;
        }
        // A chunk that starts with stray lines has no earlier line of its own to take a time from.
        if let Some(prev) = self.entries.last().map(|e| e.time) {
            for e in entries.iter_mut().take_while(|e| !e.parsed()) {
                e.time = prev;
            }
        }
        self.entries.append(&mut entries);
    }
}

/// What the app hands the tab each frame, besides the view itself.
pub struct LogFeed<'a> {
    /// What this window logged since it started, oldest first.
    pub live: &'a [Entry],
    /// When the first of `live` was logged. The file holds those lines too, so only older file lines are listed.
    pub live_since: Option<i64>,
    /// How many entries `live` has ever been given; it is capped, so its length alone does not say it changed.
    pub live_pushed: u64,
    pub state_dir: Option<&'a Path>,
}

fn clock(unix: i64) -> String {
    let secs = unix.rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", secs / 3_600, secs % 3_600 / 60, secs % 60)
}

/// One list row: time, dot, target, message, fields. A stray line (a panic, stderr noise) is only its text.
fn line(e: &Entry, font: &egui::FontId, dark: bool) -> LayoutJob {
    let p = tokens(dark);
    let mut job = LayoutJob::default();
    let mut part = |text: String, color: Color32| job.append(&text, 0.0, TextFormat { font_id: font.clone(), color, ..Default::default() });
    part(format!("{} · ", clock(e.time)), p.text_low);
    if !e.target.is_empty() {
        part(format!("{} ", e.target), p.text_low);
    }
    let message = match e.level {
        Some(Level::Error) => p.danger,
        Some(Level::Warn) => p.text_high,
        Some(Level::Debug | Level::Trace) => p.text_low,
        _ => p.text,
    };
    part(e.message.replace('\n', "\\n"), message);
    for (k, v) in &e.fields {
        part(format!(" {k}={}", v.replace('\n', "\\n")), p.text_low);
    }
    job
}
