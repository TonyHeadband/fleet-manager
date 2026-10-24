use std::fmt::{self, Write as _};
use std::fs::{File, OpenOptions};
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::filter::Targets;
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
    /// Every event logged after this call.
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

/// One layer rather than three: the event is turned into an `Entry` once and
/// handed to the file, stderr and fan-out sinks.
struct Sink {
    shared: Arc<Shared>,
    file: Option<Mutex<File>>,
    stderr: bool,
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
        } else {
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

impl<S: Subscriber> Layer<S> for Sink {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut c = Collect::default();
        event.record(&mut c);
        let meta = event.metadata();
        let entry = Entry {
            time: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64),
            level: Some(level_of(meta.level())),
            target: meta.target().to_string(),
            message: c.message,
            fields: c.fields,
        };

        let mut line = render(&entry);
        line.push('\n');
        // Logging must never take the app down: write errors are dropped.
        if let Some(file) = &self.file {
            let _ = lock(file).write_all(line.as_bytes());
        }
        if self.stderr {
            let _ = std::io::stderr().lock().write_all(line.as_bytes());
        }
        lock(&self.shared.senders).retain(|tx| tx.send(entry.clone()).is_ok());
    }
}

fn open(path: &PathBuf) -> std::io::Result<File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    OpenOptions::new().append(true).create(true).open(path)
}

fn env_filter() -> Targets {
    // A typo in FLEET_LOG must not take logging (or the process) down.
    std::env::var("FLEET_LOG")
        .ok()
        .and_then(|v| v.parse().ok())
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
    let (file, file_error) = match cfg.file.as_ref().map(open) {
        Some(Ok(f)) => (Some(Mutex::new(f)), None),
        Some(Err(e)) => (None, Some(e.to_string())),
        None => (None, None),
    };
    let shared = Arc::new(Shared {
        senders: Mutex::new(Vec::new()),
        file_error,
    });
    let sink = Sink {
        shared: shared.clone(),
        file,
        stderr: cfg.stderr == Stderr::IfTerminal && std::io::stderr().is_terminal(),
    };
    let subscriber = Registry::default().with(filter).with(sink);
    (subscriber, LogHandle { shared })
}

pub fn init(cfg: LogConfig) -> LogHandle {
    let (subscriber, handle) = build(cfg);
    // A second init (tests, embedding) keeps the first subscriber.
    let _ = tracing::subscriber::set_global_default(subscriber);
    let _ = tracing_log::LogTracer::init();
    handle
}
