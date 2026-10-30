//! Settings → Log in the app: `gui.log` and `daemon.log` read through egui's follower, and this window's own lines
//! live, kept from startup as egui's app keeps them.
use crate::bridge::{Owner, Paths};
use fleet_log::follow::{read_new, read_tail, FileLog};
use fleet_log::{render, Entry, Level, LogHandle};
use serde::Serialize;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

/// egui's `LOG_CAP`: the live lines kept, the oldest dropped.
pub const LOG_CAP: usize = 5_000;

/// One log line for the page: the parts a row draws, and the whole rendered line Search and the hover use.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LogEntryDto {
    pub time: i64,
    /// `None` for a line that is not in the log format (a panic, stderr noise).
    pub level: Option<&'static str>,
    pub target: String,
    pub message: String,
    pub fields: Vec<(String, String)>,
    pub line: String,
}

impl From<&Entry> for LogEntryDto {
    fn from(e: &Entry) -> Self {
        LogEntryDto {
            time: e.time,
            level: e.level.map(|l| match l {
                Level::Error => "error",
                Level::Warn => "warn",
                Level::Info => "info",
                Level::Debug => "debug",
                Level::Trace => "trace",
            }),
            target: e.target.clone(),
            message: e.message.clone(),
            fields: e.fields.clone(),
            line: render(e),
        }
    }
}

/// A file's lines and why it shows none, when it does not.
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LogFileDto {
    pub entries: Vec<LogEntryDto>,
    pub notice: Option<String>,
}

fn dto(file: &FileLog, what: &str) -> LogFileDto {
    LogFileDto {
        entries: file.entries.iter().map(LogEntryDto::from).collect(),
        notice: file.notice(what),
    }
}

/// What `daemon.log` changed since the last answer, for the page to splice in, as egui appends: after a reset (the
/// file shrank, grew past the tail, or cannot be read) everything shown; otherwise the held-back last line dropped
/// (`pop`) when it came back whole, then the new lines.
#[derive(Serialize, Debug, PartialEq)]
pub struct LogDelta {
    pub reset: bool,
    pub pop: bool,
    pub appended: Vec<LogEntryDto>,
    pub notice: Option<String>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct LogOpened {
    pub app: LogFileDto,
    pub daemon: LogFileDto,
}

#[derive(Default)]
pub struct Files {
    app: FileLog,
    daemon: FileLog,
}

/// The two files as last read.
#[derive(Default, Clone)]
pub struct LogFiles(Arc<Mutex<Files>>);

impl LogFiles {
    fn lock(&self) -> MutexGuard<'_, Files> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// egui's `start`: both files' tails, an unterminated last line shown.
    pub fn open(&self, state_dir: &Path) -> LogOpened {
        let mut guard = self.lock();
        let f = &mut *guard;
        for (file, name) in [(&mut f.app, "gui.log"), (&mut f.daemon, "daemon.log")] {
            let path = state_dir.join(name);
            file.load(read_tail(&path));
            file.path = Some(path);
        }
        LogOpened {
            app: dto(&f.app, "app"),
            daemon: dto(&f.daemon, "daemon"),
        }
    }

    /// egui's `poll`: what `daemon.log` gained; `None` when nothing a row or notice shows changed.
    pub fn poll(&self) -> Option<LogDelta> {
        let mut f = self.lock();
        let path = f.daemon.path.clone()?;
        let (shown, held_back) = (f.daemon.entries.len(), f.daemon.partial);
        // How many of the page's lines stay: none after a reset, all but a held-back line that a read with new lines
        // brings back whole (`FileLog::apply`), else all.
        let (changed, reset, kept) = match read_new(&path, f.daemon.offset) {
            Ok(update) if update.reset => (f.daemon.apply(update), true, 0),
            Ok(update) => {
                let grew = !update.chunk.entries.is_empty();
                let kept = shown - usize::from(grew && held_back);
                (f.daemon.apply(update), false, kept)
            }
            Err(e) => (f.daemon.fail(e), true, 0),
        };
        changed.then(|| LogDelta {
            reset,
            pop: kept < shown && !reset,
            appended: f.daemon.entries[kept..]
                .iter()
                .map(LogEntryDto::from)
                .collect(),
            notice: f.daemon.notice("daemon"),
        })
    }
}

struct Live {
    entries: VecDeque<Entry>,
    /// How many were ever pushed: the last one's `seq`, so the page can tell an event from a line it already has.
    pushed: u64,
    file_error: Option<String>,
}

/// The live log as the page first takes it: the lines, and the `seq` of the last one.
#[derive(Serialize, Debug, PartialEq)]
pub struct LiveSnapshot {
    pub entries: Vec<LogEntryDto>,
    pub seq: u64,
}

/// One live line as an event: its place in the live log, and the line.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct LiveEntry {
    pub seq: u64,
    pub entry: LogEntryDto,
}

/// What this window logged since it started, capped at `LOG_CAP`.
#[derive(Clone)]
pub struct LiveLog(Arc<Mutex<Live>>);

impl LiveLog {
    pub fn new(file_error: Option<String>) -> LiveLog {
        LiveLog(Arc::new(Mutex::new(Live {
            entries: VecDeque::new(),
            pushed: 0,
            file_error,
        })))
    }

    fn lock(&self) -> MutexGuard<'_, Live> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Keeps `e` and gives its `seq`.
    pub fn push(&self, e: Entry) -> u64 {
        let mut live = self.lock();
        live.entries.push_back(e);
        if live.entries.len() > LOG_CAP {
            live.entries.pop_front();
        }
        live.pushed += 1;
        live.pushed
    }

    pub fn snapshot(&self) -> LiveSnapshot {
        let live = self.lock();
        LiveSnapshot {
            entries: live.entries.iter().map(LogEntryDto::from).collect(),
            seq: live.pushed,
        }
    }

    pub fn file_error(&self) -> Option<String> {
        self.lock().file_error.clone()
    }
}

/// Takes this window's lines from `handle` as they are logged: kept in `live`, and each handed to `each` (the page's
/// `log-entry` event). Subscribed at once, so nothing logged before the window is up is missed.
pub fn follow(handle: &LogHandle, live: LiveLog) -> impl FnOnce(Box<dyn Fn(LiveEntry) + Send>) {
    let rx = handle.subscribe();
    move |each| {
        std::thread::spawn(move || {
            for e in rx {
                // Kept before it is sent, so a snapshot taken meanwhile already has it and its `seq`.
                let entry = LogEntryDto::from(&e);
                let seq = live.push(e);
                each(LiveEntry { seq, entry });
            }
        });
    }
}

/// The page's notice as a log line, as egui's notices are tracing events with `notify`.
/// A notice about an agent carries it (`agent_id`), as egui's do.
pub fn notice(kind: &str, text: &str, agent: Option<&str>) {
    macro_rules! at {
        ($level:ident, $notify:literal) => {
            match agent {
                Some(id) => tracing::$level!(notify = $notify, agent_id = %id, "{text}"),
                None => tracing::$level!(notify = $notify, "{text}"),
            }
        };
    }
    match kind {
        "error" => at!(error, "error"),
        "warning" => at!(warn, "warning"),
        "success" => at!(info, "success"),
        _ => at!(info, "info"),
    }
}

/// The state directory the logs are in: the connected daemon's, else the config's.
fn state_dir(paths: &Paths, owner: &Owner) -> std::path::PathBuf {
    owner
        .connected()
        .map(|c| c.state_dir)
        .unwrap_or_else(|| paths.load().state_dir)
}

/// Reads the files off the async runtime's workers.
#[tauri::command]
pub async fn log_open(
    paths: tauri::State<'_, Paths>,
    owner: tauri::State<'_, Owner>,
    files: tauri::State<'_, LogFiles>,
) -> Result<LogOpened, String> {
    let (dir, files) = (state_dir(&paths, &owner), files.inner().clone());
    tauri::async_runtime::spawn_blocking(move || files.open(&dir))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn log_poll(files: tauri::State<'_, LogFiles>) -> Result<Option<LogDelta>, String> {
    let files = files.inner().clone();
    tauri::async_runtime::spawn_blocking(move || files.poll())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn log_live(live: tauri::State<'_, LiveLog>) -> LiveSnapshot {
    live.snapshot()
}

#[tauri::command]
pub fn log_notice(kind: String, text: String, agent_id: Option<String>) {
    notice(&kind, &text, agent_id.as_deref());
}

#[tauri::command]
pub fn log_file_error(live: tauri::State<'_, LiveLog>) -> Option<String> {
    live.file_error()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fleet_log::Notice;

    const LINE: &str = "2026-10-24T10:00:01Z INFO  fleet_gui started\n";

    #[test]
    fn log_open_reads_both_tails_and_says_which_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("gui.log"), LINE).unwrap();
        let opened = LogFiles::default().open(dir.path());
        assert_eq!(opened.app.entries.len(), 1);
        assert_eq!(
            opened.app.entries[0].line,
            render(&fleet_log::parse_file(LINE)[0])
        );
        assert_eq!(opened.app.notice, None);
        assert!(opened.daemon.entries.is_empty());
        assert_eq!(
            opened.daemon.notice,
            Some(format!(
                "No daemon log at {}",
                dir.path().join("daemon.log").display()
            ))
        );
    }

    #[test]
    fn log_poll_returns_nothing_when_the_file_did_not_grow_and_the_new_lines_when_it_did() {
        let dir = tempfile::tempdir().unwrap();
        let files = LogFiles::default();
        assert_eq!(files.poll(), None, "not opened yet");
        files.open(dir.path());
        assert_eq!(files.poll(), None, "still missing");
        let daemon = dir.path().join("daemon.log");
        std::fs::write(&daemon, LINE).unwrap();
        assert_eq!(files.poll().unwrap().appended.len(), 1);
        assert_eq!(files.poll(), None, "nothing new");
        std::fs::write(
            &daemon,
            format!("{LINE}2026-10-24T10:00:02Z WARN  fleet_daemon next\n"),
        )
        .unwrap();
        // Only what is new crosses to the page (#422).
        let polled = files.poll().unwrap();
        assert!(!polled.reset && !polled.pop);
        assert_eq!(polled.appended.len(), 1);
        assert_eq!(polled.appended[0].level, Some("warn"));
    }

    #[test]
    fn a_held_back_line_that_comes_back_whole_replaces_the_shown_fragment() {
        let dir = tempfile::tempdir().unwrap();
        let daemon = dir.path().join("daemon.log");
        std::fs::write(
            &daemon,
            format!("{LINE}2026-10-24T10:00:02Z WARN  fleet_daemon hal"),
        )
        .unwrap();
        let files = LogFiles::default();
        let opened = files.open(dir.path());
        assert_eq!(
            opened.daemon.entries.len(),
            2,
            "the fragment shows on the first read"
        );
        std::fs::write(
            &daemon,
            format!("{LINE}2026-10-24T10:00:02Z WARN  fleet_daemon half done\n"),
        )
        .unwrap();
        let polled = files.poll().unwrap();
        assert!(!polled.reset);
        assert!(polled.pop, "the fragment goes");
        assert_eq!(polled.appended.len(), 1);
        assert_eq!(polled.appended[0].message, "half done");
    }

    #[test]
    fn a_truncated_file_starts_the_page_over() {
        let dir = tempfile::tempdir().unwrap();
        let daemon = dir.path().join("daemon.log");
        std::fs::write(
            &daemon,
            format!("{LINE}2026-10-24T10:00:02Z WARN  fleet_daemon next\n"),
        )
        .unwrap();
        let files = LogFiles::default();
        files.open(dir.path());
        std::fs::write(&daemon, "2026-10-24T11:00:00Z INFO  fleet_daemon fresh\n").unwrap();
        let polled = files.poll().unwrap();
        assert!(polled.reset);
        assert_eq!(polled.appended.len(), 1);
        assert_eq!(polled.appended[0].message, "fresh");
        // Gone altogether: the page empties and says why.
        std::fs::remove_file(&daemon).unwrap();
        let polled = files.poll().unwrap();
        assert!(polled.reset && polled.appended.is_empty());
        assert!(polled.notice.unwrap().starts_with("No daemon log at"));
    }

    #[test]
    fn the_live_log_keeps_the_last_5000() {
        let live = LiveLog::new(None);
        for n in 0..=LOG_CAP {
            live.push(Entry {
                time: n as i64,
                level: Some(Level::Info),
                target: "t".into(),
                message: format!("m{n}"),
                fields: vec![],
            });
        }
        let snapshot = live.snapshot();
        assert_eq!(snapshot.entries.len(), LOG_CAP);
        assert_eq!(snapshot.entries[0].message, "m1", "the oldest dropped");
        assert_eq!(snapshot.seq, LOG_CAP as u64 + 1, "every line pushed counts");
    }

    #[test]
    fn a_notice_is_logged_with_its_notify_field() {
        let (subscriber, handle) = fleet_log::build(fleet_log::LogConfig {
            file: None,
            stderr: fleet_log::Stderr::Never,
        });
        let rx = handle.subscribe();
        tracing::subscriber::with_default(subscriber, || {
            notice("warning", "Could not save the theme", None);
            notice("success", "Repo granted", Some("a1"));
        });
        let e = rx.try_recv().unwrap();
        assert_eq!(
            (e.level, e.notice()),
            (Some(Level::Warn), Some(Notice::Warning))
        );
        assert_eq!(e.message, "Could not save the theme");
        assert!(e.fields.iter().all(|(k, _)| k != "agent_id"));
        // A notice about an agent carries it, as egui's do (`agent_id = %id`), so Search finds it.
        let e = rx.try_recv().unwrap();
        assert!(
            e.fields.contains(&("agent_id".into(), "a1".into())),
            "{:?}",
            e.fields
        );
    }
}
