use std::fmt::{self, Write as _};
use std::fs::{File, OpenOptions};
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_log::NormalizeEvent;
use tracing_subscriber::filter::{filter_fn, FilterExt, Targets};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::Registry;
use tracing_subscriber::Layer;

use crate::{render, Entry, Level};

pub const DEFAULT_FILTER: &str =
    "warn,fleet_daemon=info,fleet_client=info,fleet_gui=info,fleet_log=info";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stderr {
    IfTerminal,
    Never,
}

#[derive(Debug, Clone)]
pub struct LogConfig {
    pub file: Option<PathBuf>,
    pub stderr: Stderr,
}

struct Shared {
    senders: Mutex<Vec<Sender<Entry>>>,
    file_error: Option<String>,
}

#[derive(Clone)]
pub struct LogHandle {
    shared: Arc<Shared>,
}

impl LogHandle {
    /// Every event logged after this call. The channel is unbounded: a receiver that is not drained keeps every
    /// entry in memory. The GUI drains its receiver every frame; a receiver that is dropped is let go of on the
    /// next event.
    pub fn subscribe(&self) -> Receiver<Entry> {
        let (tx, rx) = channel();
        lock(&self.shared.senders).push(tx);
        rx
    }

    /// Why the file layer is off, if it is.
    pub fn file_error(&self) -> Option<&str> {
        self.shared.file_error.as_deref()
    }
}

// A panic elsewhere must not silence logging for the rest of the process.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Default)]
struct Collect {
    message: String,
    fields: Vec<(String, String)>,
}

impl Collect {
    fn push(&mut self, field: &Field, value: String) {
        if field.name() == "message" {
            self.message = value;
        } else if !field.name().starts_with("log.") {
            // `log.target`, `log.file`, ... are tracing-log's bookkeeping for
            // records bridged from the `log` crate; the real values are in the
            // normalized metadata.
            self.fields.push((field.name().to_string(), value));
        }
    }
}

impl Visit for Collect {
    fn record_str(&mut self, field: &Field, value: &str) {
        // Debug would wrap strings in quotes.
        self.push(field, value.to_string());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let mut s = String::new();
        let _ = write!(s, "{value:?}");
        self.push(field, s);
    }
}

fn level_of(l: &tracing::Level) -> Level {
    match *l {
        tracing::Level::ERROR => Level::Error,
        tracing::Level::WARN => Level::Warn,
        tracing::Level::INFO => Level::Info,
        tracing::Level::DEBUG => Level::Debug,
        tracing::Level::TRACE => Level::Trace,
    }
}

/// The one event -> `Entry` conversion shared by every layer.
fn entry_of(event: &Event<'_>) -> Entry {
    let mut c = Collect::default();
    event.record(&mut c);
    // Events from the `log` crate arrive with target "log"; this restores the
    // record's real target and level.
    let normalized = event.normalized_metadata();
    let meta = normalized.as_ref().unwrap_or_else(|| event.metadata());
    Entry {
        time: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64),
        level: Some(level_of(meta.level())),
        target: meta.target().to_string(),
        message: c.message,
        fields: c.fields,
    }
}

fn line_of(entry: &Entry) -> String {
    let mut line = render(entry);
    line.push('\n');
    line
}

struct FileLayer {
    file: Mutex<File>,
}

impl<S: Subscriber> Layer<S> for FileLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let line = line_of(&entry_of(event));
        // Logging must never take the app down: write errors are dropped.
        let _ = lock(&self.file).write_all(line.as_bytes());
    }
}

struct StderrLayer {
    out: Mutex<Box<dyn Write + Send>>,
}

impl<S: Subscriber> Layer<S> for StderrLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let _ = lock(&self.out).write_all(line_of(&entry_of(event)).as_bytes());
    }
}

/// Where the stderr layer writes, and whether that is a terminal. The process's stderr outside tests.
#[doc(hidden)]
pub struct StderrSink {
    pub writer: Box<dyn Write + Send>,
    pub is_terminal: bool,
}

struct FanOutLayer {
    shared: Arc<Shared>,
}

impl<S: Subscriber> Layer<S> for FanOutLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut senders = lock(&self.shared.senders);
        if senders.is_empty() {
            return;
        }
        let entry = entry_of(event);
        senders.retain(|tx| tx.send(entry.clone()).is_ok());
    }
}

fn open(path: &PathBuf) -> std::io::Result<File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    OpenOptions::new().append(true).create(true).open(path)
}

fn env_filter() -> Targets {
    filter_from(std::env::var("FLEET_LOG").ok().as_deref())
}

// A typo in FLEET_LOG must not take logging (or the process) down.
fn filter_from(spec: Option<&str>) -> Targets {
    spec.and_then(|v| v.parse().ok())
        .unwrap_or_else(default_filter)
}

fn default_filter() -> Targets {
    DEFAULT_FILTER.parse().expect("DEFAULT_FILTER is valid")
}

/// Builds the subscriber without installing it; tests use `with_default`.
pub fn build(cfg: LogConfig) -> (impl Subscriber + Send + Sync, LogHandle) {
    build_with_filter(cfg, env_filter())
}

/// Like `build` with an explicit filter, so callers need not read `FLEET_LOG`.
pub fn build_with_filter(
    cfg: LogConfig,
    filter: Targets,
) -> (impl Subscriber + Send + Sync, LogHandle) {
    let sink = StderrSink {
        writer: Box::new(std::io::stderr()),
        is_terminal: std::io::stderr().is_terminal(),
    };
    build_with_sink(cfg, filter, sink)
}

/// Like `build_with_filter`, writing what would go to stderr to `sink`, for tests.
#[doc(hidden)]
pub fn build_with_sink(
    cfg: LogConfig,
    filter: Targets,
    sink: StderrSink,
) -> (impl Subscriber + Send + Sync, LogHandle) {
    let (file, file_error) = match cfg.file.as_ref().map(open) {
        Some(Ok(f)) => (Some(Mutex::new(f)), None),
        Some(Err(e)) => (None, Some(e.to_string())),
        None => (None, None),
    };
    let shared = Arc::new(Shared {
        senders: Mutex::new(Vec::new()),
        file_error,
    });
    let stderr = (cfg.stderr == Stderr::IfTerminal && sink.is_terminal).then(|| StderrLayer {
        out: Mutex::new(sink.writer),
    });
    // FLEET_LOG tunes what the files and the terminal record; the footer reads
    // the fan-out, so our own info-and-above always reaches it whatever the
    // filter says, or a narrow FLEET_LOG would hide errors from the user.
    let notices = filter_fn(|meta| {
        meta.target().starts_with("fleet_") && *meta.level() <= tracing::Level::INFO
    });
    let subscriber = Registry::default()
        .with(file.map(|file| FileLayer { file }.with_filter(filter.clone())))
        .with(
            FanOutLayer {
                shared: shared.clone(),
            }
            .with_filter(filter.clone().or(notices)),
        )
        .with(stderr.map(|layer| layer.with_filter(filter)));
    (subscriber, LogHandle { shared })
}

pub fn init(cfg: LogConfig) -> LogHandle {
    let (subscriber, handle) = build(cfg);
    // A second init (tests, embedding) keeps the first subscriber.
    let _ = tracing::subscriber::set_global_default(subscriber);
    let _ = tracing_log::LogTracer::init();
    handle
}

/// What a panic says, for the log and for a message box: the payload and where it happened.
pub fn panic_text(info: &std::panic::PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    let message = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("(no message)");
    match info.location() {
        Some(l) => format!("{message} at {}:{}", l.file(), l.line()),
        None => message.to_string(),
    }
}

/// Logs every panic as an error, then runs the hook that was installed before (the default one prints to
/// stderr). A GUI exe has no stderr, so without this a panic leaves no trace.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        tracing::error!(
            thread = thread.name().unwrap_or("unnamed"),
            "panic: {}",
            panic_text(info)
        );
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_from_falls_back_to_the_default() {
        let info = tracing::Level::INFO;
        assert!(filter_from(None).would_enable("fleet_gui", &info));
        assert!(!filter_from(None).would_enable("winit", &info));
        assert!(filter_from(Some("winit=info")).would_enable("winit", &info));
        // Unparsable spec: default, not a panic.
        assert!(!filter_from(Some("=!=garbage==")).would_enable("winit", &info));
        assert!(filter_from(Some("=!=garbage==")).would_enable("fleet_gui", &info));
    }
}
