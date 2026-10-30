//! Settings → Log: the app's and the daemon's log lines in one list, from `gui.log` and `daemon.log` in the state
//! directory plus what this window logged since it started.
use crate::theme::{mono, settings, size, tokens, tone_color};
use crate::widgets::{chip, text_dot, toggle};
use eframe::egui::{self, Color32, RichText, ScrollArea, TextEdit, TextFormat, Ui};
use fleet_api::Tone;
use fleet_log::follow::{read_new, read_tail, Chunk, FileLog, FileState, Update};
pub use fleet_log::follow::{tail, TAIL};
use fleet_log::{render, Entry, Level};
use std::io;
use std::path::Path;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

/// How often the daemon's file is checked for new lines while the tab is on screen.
const POLL: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    App,
    Daemon,
    Both,
}

enum Msg {
    Loaded {
        app: io::Result<Chunk>,
        daemon: io::Result<Chunk>,
    },
    Polled(io::Result<Update>),
}

#[derive(PartialEq)]
struct Key {
    version: u64,
    pushed: u64,
    source: Source,
    levels: [bool; 4],
}

/// One line of the list: an index into the app's lines (file then live) or into the daemon's.
#[derive(Clone, Copy)]
struct Row {
    daemon: bool,
    i: usize,
}

struct Built {
    key: Key,
    /// Kept apart from `key` so a frame can compare it with the field without copying it.
    search: String,
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
            let _ = tx.send(Msg::Loaded {
                app: read_tail(&app),
                daemon: read_tail(&daemon),
            });
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
        let Some(path) = self.daemon.path.clone() else {
            return;
        };
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
        let notices = [
            (self.source != Source::Daemon)
                .then(|| self.app.notice("app"))
                .flatten(),
            (self.source != Source::App)
                .then(|| self.daemon.notice("daemon"))
                .flatten(),
        ];
        for notice in notices.into_iter().flatten() {
            ui.label(RichText::new(notice).size(size::SMALL).color(p.text_low));
        }
        let Some(built) = &self.built else { return };
        let app_at = |i: usize| {
            if i < built.app_file {
                &self.app.entries[i]
            } else {
                &pane.live[i - built.app_file]
            }
        };
        let font = mono(size::CODE);
        // A row is a `horizontal`, never shorter than a button; show_rows must lay out the height a row really has.
        let height = ui
            .fonts_mut(|f| f.row_height(&font))
            .max(ui.spacing().interact_size.y);
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show_rows(ui, height, built.rows.len(), |ui, range| {
                for row in &built.rows[range] {
                    let entry = if row.daemon {
                        &self.daemon.entries[row.i]
                    } else {
                        app_at(row.i)
                    };
                    draw_row(ui, entry, &font);
                }
            });
    }

    fn controls(&mut self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            for (source, label) in [
                (Source::App, "App"),
                (Source::Daemon, "Daemon"),
                (Source::Both, "Both"),
            ] {
                if chip(ui, label, self.source == source).clicked() {
                    self.source = source;
                }
            }
            ui.add_space(8.0);
            for (on, label) in self
                .levels
                .iter_mut()
                .zip(["error", "warning", "info", "debug"])
            {
                if toggle(ui, label, *on).clicked() {
                    *on = !*on;
                }
            }
            ui.add_space(8.0);
            ui.add(
                TextEdit::singleline(&mut self.search)
                    .hint_text("Search")
                    .desired_width(settings::LOG_SEARCH),
            );
        });
    }

    /// Rebuilds the merged, filtered list when something it depends on changed; otherwise a frame costs nothing.
    fn rebuild(&mut self, pane: &LogFeed<'_>) {
        let key = Key {
            version: self.version,
            pushed: pane.live_pushed,
            source: self.source,
            levels: self.levels,
        };
        if self
            .built
            .as_ref()
            .is_some_and(|b| b.key == key && b.search == self.search)
        {
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
        let app_at = |i: usize| {
            if i < app_file {
                &self.app.entries[i]
            } else {
                &pane.live[i - app_file]
            }
        };
        let daemon = &self.daemon.entries;
        let (app_wanted, daemon_wanted) =
            (self.source != Source::Daemon, self.source != Source::App);
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
                    rows.push(Row {
                        daemon: false,
                        i: a,
                    });
                    a += 1;
                }
                (false, true) => {
                    rows.push(Row { daemon: true, i: d });
                    d += 1;
                }
                (true, true) if app_at(a).time <= daemon[d].time => {
                    rows.push(Row {
                        daemon: false,
                        i: a,
                    });
                    a += 1;
                }
                (true, true) => {
                    rows.push(Row { daemon: true, i: d });
                    d += 1;
                }
            }
        }
        self.built = Some(Built {
            key,
            search: self.search.clone(),
            rows,
            app_file,
        });
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
        live[from..]
            .iter()
            .take_while(|l| l.time == e.time)
            .any(|l| render(l) == line)
    };
    let Some(oldest) = live.first().map(|l| l.time) else {
        return file.len();
    };
    // Lines older than the oldest live one cannot match; skipping them saves rendering most of a large file.
    let older = file.partition_point(|e| e.time < oldest);
    older
        + file[older..]
            .iter()
            .position(in_live)
            .unwrap_or(file.len() - older)
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
    format!(
        "{:02}:{:02}:{:02}",
        secs / 3_600,
        secs % 3_600 / 60,
        secs % 60
    )
}

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
        ui.label(
            RichText::new(clock(e.time))
                .font(font.clone())
                .color(p.text_low),
        );
        match level_colour(e.level, dark) {
            Some(c) => {
                text_dot(ui, c, settings::LOG_DOT, font);
            }
            None => ui.add_space(settings::LOG_DOT),
        }
        let mut job = egui::text::LayoutJob::default();
        let mut part = |text: String, color: Color32| {
            job.append(
                &text,
                0.0,
                TextFormat {
                    font_id: font.clone(),
                    color,
                    ..Default::default()
                },
            )
        };
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
        ui.add(egui::Label::new(job).truncate())
            .on_hover_text(render(e));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(lines: &[&str]) -> Vec<Entry> {
        lines
            .iter()
            .flat_map(|l| fleet_log::parse_file(&format!("{l}\n")))
            .collect()
    }

    const PRE: &str = "2026-10-24T10:00:01Z INFO  fleet_gui gui started";
    const A: &str = "2026-10-24T10:00:01Z INFO  fleet_gui a";
    const B: &str = "2026-10-24T10:00:02Z INFO  fleet_gui b";
    const C: &str = "2026-10-24T10:00:03Z INFO  fleet_gui c";

    #[test]
    fn a_file_line_from_the_same_second_as_the_first_live_one_is_listed() {
        // "gui started" was logged before the window subscribed, in the second of the first live line.
        assert_eq!(
            file_lines_before_live(&parse(&[PRE, A, B]), &parse(&[A, B])),
            1
        );
    }

    #[test]
    fn file_lines_the_capped_live_log_dropped_are_listed() {
        assert_eq!(
            file_lines_before_live(&parse(&[PRE, A, B, C]), &parse(&[B, C])),
            2
        );
    }

    #[test]
    fn a_file_that_shares_no_line_with_the_live_log_is_listed_whole() {
        assert_eq!(
            file_lines_before_live(&parse(&[PRE, A]), &parse(&[B, C])),
            2
        );
        assert_eq!(file_lines_before_live(&parse(&[PRE, A]), &[]), 2);
    }

    #[test]
    fn a_live_notice_the_file_filtered_out_does_not_hide_the_overlap() {
        let notice = "2026-10-24T10:00:01Z ERROR fleet_gui shown in the footer only";
        assert_eq!(
            file_lines_before_live(&parse(&[PRE, A, B]), &parse(&[notice, A, B])),
            1
        );
    }
}
