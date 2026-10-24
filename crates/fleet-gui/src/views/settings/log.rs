//! Settings → Log: the app's and the daemon's log lines in one list, from `gui.log` and `daemon.log` in the state
//! directory plus what this window logged since it started.
use crate::theme::{mono, size, tokens, tone_color};
use crate::widgets::{chip, text_dot, toggle};
use eframe::egui::{self, Color32, RichText, ScrollArea, TextEdit, TextFormat, Ui};
use fleet_api::Tone;
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
    /// The last entry is an unterminated line, shown now and replaced when its newline arrives.
    partial: bool,
}

/// `show_partial` also lists an unterminated last line: on the first read it is often all a crash left behind.
fn lines(bytes: &[u8], end: u64, show_partial: bool) -> Chunk {
    let keep = bytes.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    let partial = show_partial && keep < bytes.len();
    let shown = if partial { bytes.len() } else { keep };
    let entries = fleet_log::parse_file(&String::from_utf8_lossy(&bytes[..shown]));
    Chunk { entries, end: end - (bytes.len() - keep) as u64, partial }
}

fn whole_lines(bytes: &[u8], end: u64) -> Chunk {
    lines(bytes, end, false)
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

#[derive(PartialEq, Eq)]
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
    /// The last entry is an unterminated line that the next read starts over from.
    partial: bool,
}

impl FileLog {
    fn new() -> FileLog {
        FileLog { path: None, entries: Vec::new(), offset: 0, state: FileState::Pending, partial: false }
    }

    fn load(&mut self, result: io::Result<Chunk>) {
        match result {
            Ok(chunk) => {
                self.entries = chunk.entries;
                self.offset = chunk.end;
                self.partial = chunk.partial;
                self.state = FileState::Read;
            }
            Err(e) => {
                self.fail(e);
            }
        }
    }

    /// Whether anything a row or notice shows changed.
    fn fail(&mut self, e: io::Error) -> bool {
        let state = if e.kind() == io::ErrorKind::NotFound { FileState::Missing } else { FileState::Failed(e.to_string()) };
        let changed = !self.entries.is_empty() || self.state != state;
        self.entries.clear();
        self.offset = 0;
        self.partial = false;
        self.state = state;
        changed
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
    /// File reads started (the first load and each poll), for tests that check a hidden tab reads nothing.
    reads: u64,
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
            reads: 0,
            tx,
            rx,
        }
    }

    pub fn source(&self) -> Source {
        self.source
    }

    #[doc(hidden)]
    pub fn reads(&self) -> u64 {
        self.reads
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
        self.reads += 1;
        std::thread::spawn(move || {
            let read = |p: &Path| tail_bytes(p, TAIL).map(|(bytes, end)| lines(&bytes, end, true));
            let _ = tx.send(Msg::Loaded { app: read(&app), daemon: read(&daemon) });
            ctx.request_repaint();
        });
    }

    fn receive(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            // A poll that found nothing leaves the version alone, or the list would rebuild every second.
            let changed = match msg {
                Msg::Loaded { app, daemon } => {
                    self.app.load(app);
                    self.daemon.load(daemon);
                    self.loaded = true;
                    self.last_poll = Instant::now();
                    true
                }
                Msg::Polled(result) => {
                    self.polling = false;
                    self.last_poll = Instant::now();
                    match result {
                        Ok(update) => self.daemon.apply(update),
                        Err(e) => self.daemon.fail(e),
                    }
                }
            };
            self.version += u64::from(changed);
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
        self.reads += 1;
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
        // A row is a `horizontal`, never shorter than a button; show_rows must lay out the height a row really has.
        let height = ui.fonts_mut(|f| f.row_height(&font)).max(ui.spacing().interact_size.y);
        ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show_rows(ui, height, built.rows.len(), |ui, range| {
            for row in &built.rows[range] {
                let entry = if row.daemon { &self.daemon.entries[row.i] } else { app_at(row.i) };
                draw_row(ui, entry, &font);
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
        let key = Key { version: self.version, pushed: pane.live_pushed, source: self.source, levels: self.levels, search: self.search.clone() };
        if self.built.as_ref().is_some_and(|b| b.key == key) {
            return;
        }
        let app_file = file_lines_before_live(&self.app.entries, pane.live);
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

/// How many of `gui.log`'s lines to list before the live ones: those up to the first that the live log also holds.
/// By identity (time and rendered line), not by time: a line logged in the same second just before the window
/// subscribed is in the file only, and lines the capped live log dropped are in the file only too. The live log
/// can hold notices the file's filter left out, so any live line may be the first match. Live lines are sorted by
/// time, so each file line is compared with those of its own second only.
fn file_lines_before_live(file: &[Entry], live: &[Entry]) -> usize {
    let in_live = |e: &Entry| {
        let from = live.partition_point(|l| l.time < e.time);
        let line = render(e);
        live[from..].iter().take_while(|l| l.time == e.time).any(|l| render(l) == line)
    };
    let Some(oldest) = live.first().map(|l| l.time) else { return file.len() };
    // Lines older than the oldest live one cannot match; skipping them saves rendering most of a large file.
    let older = file.partition_point(|e| e.time < oldest);
    older + file[older..].iter().position(in_live).unwrap_or(file.len() - older)
}

impl FileLog {
    /// Whether anything a row or notice shows changed.
    fn apply(&mut self, update: Update) -> bool {
        let Update { reset, chunk } = update;
        let was_read = self.state == FileState::Read;
        self.state = FileState::Read;
        let mut entries = chunk.entries;
        if reset {
            self.offset = chunk.end;
            self.partial = chunk.partial;
            self.entries = entries;
            return true;
        }
        if entries.is_empty() {
            // Still nothing whole after the held-back line: it stays as it is.
            self.offset = chunk.end;
            return !was_read;
        }
        // The read starts at the unterminated line shown earlier, which comes back whole now.
        if std::mem::take(&mut self.partial) {
            self.entries.pop();
        }
        // A chunk that starts with stray lines has no earlier line of its own to take a time from.
        if let Some(prev) = self.entries.last().map(|e| e.time) {
            for e in entries.iter_mut().take_while(|e| !e.parsed()) {
                e.time = prev;
            }
        }
        self.offset = chunk.end;
        self.partial = chunk.partial;
        self.entries.append(&mut entries);
        true
    }
}

/// What the app hands the tab each frame, besides the view itself.
pub struct LogFeed<'a> {
    /// What this window logged since it started, oldest first.
    pub live: &'a [Entry],
    /// How many entries `live` has ever been given; it is capped, so its length alone does not say it changed.
    pub live_pushed: u64,
    pub state_dir: Option<&'a Path>,
}

fn clock(unix: i64) -> String {
    let secs = unix.rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", secs / 3_600, secs % 3_600 / 60, secs % 60)
}

const DOT: f32 = 7.0;

/// The level's dot, in the footer's colours. A stray line has none.
fn level_colour(level: Option<Level>, dark: bool) -> Option<Color32> {
    let p = tokens(dark);
    Some(match level? {
        Level::Error => p.danger,
        Level::Warn => tone_color(Tone::Attention, dark),
        Level::Info => p.text_low,
        Level::Debug | Level::Trace => p.border,
    })
}

/// One list row: time, level dot, then target, message and fields. A stray line (a panic, stderr noise) has no dot
/// but keeps its place, so the text stays aligned. The full line is the hover text because a long row is cut off.
fn draw_row(ui: &mut Ui, e: &Entry, font: &egui::FontId) {
    let dark = ui.visuals().dark_mode;
    let p = tokens(dark);
    ui.horizontal(|ui| {
        ui.label(RichText::new(clock(e.time)).font(font.clone()).color(p.text_low));
        match level_colour(e.level, dark) {
            Some(c) => {
                text_dot(ui, c, DOT, font);
            }
            None => ui.add_space(DOT),
        }
        let mut job = egui::text::LayoutJob::default();
        let mut part = |text: String, color: Color32| job.append(&text, 0.0, TextFormat { font_id: font.clone(), color, ..Default::default() });
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
        ui.add(egui::Label::new(job).truncate()).on_hover_text(render(e));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(entries: Vec<Entry>, end: u64) -> Update {
        Update { reset: false, chunk: Chunk { entries, end, partial: false } }
    }

    fn parse(lines: &[&str]) -> Vec<Entry> {
        lines.iter().flat_map(|l| fleet_log::parse_file(&format!("{l}\n"))).collect()
    }

    const PRE: &str = "2026-10-24T10:00:01Z INFO  fleet_gui gui started";
    const A: &str = "2026-10-24T10:00:01Z INFO  fleet_gui a";
    const B: &str = "2026-10-24T10:00:02Z INFO  fleet_gui b";
    const C: &str = "2026-10-24T10:00:03Z INFO  fleet_gui c";

    #[test]
    fn a_file_line_from_the_same_second_as_the_first_live_one_is_listed() {
        // "gui started" was logged before the window subscribed, in the second of the first live line.
        assert_eq!(file_lines_before_live(&parse(&[PRE, A, B]), &parse(&[A, B])), 1);
    }

    #[test]
    fn file_lines_the_capped_live_log_dropped_are_listed() {
        assert_eq!(file_lines_before_live(&parse(&[PRE, A, B, C]), &parse(&[B, C])), 2);
    }

    #[test]
    fn a_file_that_shares_no_line_with_the_live_log_is_listed_whole() {
        assert_eq!(file_lines_before_live(&parse(&[PRE, A]), &parse(&[B, C])), 2);
        assert_eq!(file_lines_before_live(&parse(&[PRE, A]), &[]), 2);
    }

    #[test]
    fn a_live_notice_the_file_filtered_out_does_not_hide_the_overlap() {
        let notice = "2026-10-24T10:00:01Z ERROR fleet_gui shown in the footer only";
        assert_eq!(file_lines_before_live(&parse(&[PRE, A, B]), &parse(&[notice, A, B])), 1);
    }

    #[test]
    fn a_poll_that_finds_nothing_changes_nothing() {
        let mut file = FileLog::new();
        file.load(Ok(Chunk { entries: fleet_log::parse_file("2026-10-24T10:00:01Z INFO  t m\n"), end: 31, partial: false }));
        assert!(!file.apply(chunk(vec![], 31)), "nothing new");
        assert!(file.apply(chunk(fleet_log::parse_file("2026-10-24T10:00:02Z INFO  t n\n"), 62)));
        assert_eq!(file.entries.len(), 2);
        assert!(file.fail(io::Error::from(io::ErrorKind::NotFound)), "the lines vanish");
        assert!(!file.fail(io::Error::from(io::ErrorKind::NotFound)), "still missing");
    }
}
