use fleet_daemon::agents::{AgentService, ServiceOptions};
use fleet_daemon::config::{self, Config};
use fleet_daemon::http::{bind_loopback, router, AppState};
use fleet_daemon::repos::Repos;
use fleet_daemon::session::SessionHost;
use fleet_daemon::store::Store;
use fleet_log::{LogConfig, Stderr};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// Set once the logger is installed; a fatal error before that has nowhere to be logged but stderr.
static LOGGER_UP: AtomicBool = AtomicBool::new(false);

const RECONCILE_EVERY: Duration = Duration::from_secs(5);
/// How long in-flight requests get after a shutdown signal before the daemon exits anyway.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

const USAGE: &str = "\
Usage: fleet-daemon [--config <path>]

Runs the fleet-manager daemon: the HTTP API on loopback, and the agents' terminals.

Options:
  --config <path>  Config file (default: ~/.config/fleet-manager/config.toml; Windows: %APPDATA%\\fleet-manager\\config.toml)
  --version        Print the version and exit
  -h, --help       Print this help and exit

Stops on Ctrl-C, SIGTERM or POST /api/shutdown; the agents stop with it and resume at the next start.";

enum Cli {
    Run { config: Option<PathBuf> },
    Help,
    Version,
}

/// Anything not understood is an error: a mistyped flag that was ignored would start the daemon on the wrong config.
fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Cli, String> {
    let mut config = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => return Ok(Cli::Help),
            "--version" => return Ok(Cli::Version),
            "--config" => match args.next() {
                Some(p) if !p.starts_with('-') => config = Some(PathBuf::from(p)),
                _ => return Err("--config needs a path".into()),
            },
            _ => return Err(format!("unknown argument {a:?}")),
        }
    }
    Ok(Cli::Run { config })
}

async fn run(config_path: PathBuf) -> Result<(), String> {
    // First, because it may move the state dir; its notes wait for the logger, which needs that dir.
    // fleet-board only ever ran on Linux: nothing to move on Windows.
    #[cfg(unix)]
    let notes = fleet_fs::migrate::migrate_legacy_dirs(&config::home_dir());
    #[cfg(not(unix))]
    let notes: Vec<String> = Vec::new();
    // A config error happens before the state dir is known, so it can only go to stderr (the app captures it).
    let cfg: Config = config::load(&config_path).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&cfg.state_dir)
        .map_err(|e| format!("cannot create {}: {e}", cfg.state_dir.display()))?;
    let log = fleet_log::init(LogConfig {
        file: Some(cfg.state_dir.join("daemon.log")),
        stderr: Stderr::IfTerminal,
    });
    LOGGER_UP.store(true, Ordering::Relaxed);
    if let Some(e) = log.file_error() {
        warn!(error = e, "daemon.log is not being written");
    }
    for note in notes {
        info!("{note}");
    }
    if cfg.legacy_repos > 0 {
        warn!(
            entries = cfg.legacy_repos,
            "config.toml still lists [[repos]], which are no longer read; add the repos to a product in the app and remove them from the file"
        );
    }
    // Each one is checked again when it is used; this only says early which user file needs fixing.
    for d in fleet_daemon::fleet::defs::Defs::new(cfg.fleet_dir.clone()).list() {
        if let Some(error) = d.error {
            warn!(definition = %d.name, %error, "fleet definition does not load");
        }
    }
    if cfg.telemetry.otlp_endpoint.is_some() {
        debug!("telemetry is not built yet; otlp_endpoint ignored");
    }
    // Bind first: the hook URLs handed to agents must carry the port actually in use.
    let listener = bind_loopback(cfg.listen).await.map_err(|e| e.to_string())?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;

    std::fs::create_dir_all(&cfg.tasks_root)
        .map_err(|e| format!("cannot create {}: {e}", cfg.tasks_root.display()))?;
    let state_dir = cfg.state_dir.clone();
    let store = Arc::new(
        Store::open(&cfg.state_dir.join("fleet.db"))
            .await
            .map_err(|e| e.to_string())?,
    );
    if store.was_reset() {
        // Nothing is deleted: a clone can hold branches that exist nowhere else.
        for path in fleet_daemon::store::leftovers(&state_dir, &cfg.tasks_root) {
            warn!(
                "left over from before products; delete it if you no longer need it: {}",
                path.display()
            );
        }
    }
    let mut opts = ServiceOptions::new(&format!("http://{addr}"));
    opts.claude = cfg.claude_command.clone();
    let repos = Arc::new(Repos::new(
        store.clone(),
        cfg.state_dir.clone(),
        cfg.tasks_root.clone(),
        config::home_dir(),
    ));
    repos.sweep_tmp_clones();
    let sessions = SessionHost::new();
    let svc = Arc::new(AgentService::new(
        cfg,
        store.clone(),
        repos,
        sessions.clone(),
        opts,
    ));
    svc.adopt_legacy_dead().await.map_err(|e| e.to_string())?;
    store.backfill_tasks().await.map_err(|e| e.to_string())?;
    // After backfill, so records made on upgrade are measured too. In the background: a big tree takes a while.
    svc.request_measure_all();
    // After backfill, so every folder with a record is known. In the background: it runs git per remembered repo and
    // must not hold up the "listening" line.
    let scanner = svc.clone();
    tokio::spawn(async move { scanner.rescan_orphans().await });
    // Each check runs a CLI with a 10 s timeout; the list shows "unchecked" until they land.
    let checker = svc.personas().clone();
    tokio::spawn(async move {
        if let Err(e) = checker.check_all().await {
            warn!(error = %e, "persona check failed");
        }
    });
    // Asks GitHub at once, then every MERGE_CHECK_EVERY and when an agent stops or reaches Review.
    svc.start_merge_watcher(fleet_daemon::merges::MERGE_CHECK_EVERY);

    let ticker = svc.clone();
    // No session outlives a daemon, so every agent not stopped was interrupted. They are relaunched beside the
    // server, not before it: each persona check can take seconds, and the resumed agents' hooks need the server.
    // The ticker starts only after that first pass, or it would find the agents not yet relaunched dead.
    let reconciler = tokio::spawn(async move {
        if let Err(e) = ticker.recover().await {
            error!(error = %e, "resuming the interrupted agents failed");
        }
        loop {
            tokio::time::sleep(RECONCILE_EVERY).await;
            if let Err(e) = ticker.reconcile().await {
                error!(error = %e, "reconcile failed");
            }
        }
    });

    // Written before the "listening" line, so a client that sees the line can rely on it. A client stops the
    // daemon through `/api/shutdown`, or SIGTERM to the pid in this file, so it holds nothing but the pid.
    let requested = std::sync::Arc::new(tokio::sync::Notify::new());
    let signal = shutdown_signal(requested.clone());
    let pidfile = state_dir.join("daemon.pid");
    std::fs::write(&pidfile, format!("{}\n", std::process::id()))
        .map_err(|e| format!("cannot write {}: {e}", pidfile.display()))?;

    info!(listen = %addr, version = env!("CARGO_PKG_VERSION"), "daemon started");
    println!("fleet-manager listening on http://{addr}");
    // Graceful shutdown waits for every open connection, and an event or terminal stream never ends on its own,
    // so a connected app or browser tab would keep the daemon alive forever. After the signal: stop accepting,
    // give in-flight requests a moment, then exit regardless.
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let stopping = svc.clone();
    let server = axum::serve(
        listener,
        router(AppState::new(svc, store, sessions.clone(), addr).with_shutdown(requested.clone())),
    )
    .with_graceful_shutdown(async move {
        let _ = stop_rx.await;
    });
    let signalled = async move {
        signal.await;
        info!("shutdown signal received");
        let _ = stop_tx.send(());
        tokio::time::sleep(SHUTDOWN_GRACE).await;
    };
    let served = tokio::select! {
        r = server => r.map_err(|e| e.to_string()),
        () = signalled => Ok(()),
    };
    // Nothing may record the sessions' end as a crash or a failed resume, and a resume pass still running must
    // start nothing more: the next start resumes them. The flag first, so a pass between awaits sees it.
    stopping.begin_shutdown();
    reconciler.abort();
    sessions.shutdown_all();
    let _ = std::fs::remove_file(&pidfile);
    if served.is_ok() {
        info!("daemon stopped");
    }
    served
}

/// Ctrl-C, SIGTERM (Unix), closing the console (Windows), or `POST /api/shutdown`. Handlers are installed when this
/// is called, not when the future is first polled: a client may ask as soon as it sees the "listening" line.
fn shutdown_signal(
    requested: std::sync::Arc<tokio::sync::Notify>,
) -> impl std::future::Future<Output = ()> {
    #[cfg(unix)]
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("SIGTERM handler");
    #[cfg(windows)]
    let mut close = tokio::signal::windows::ctrl_close().expect("console close handler");
    async move {
        #[cfg(unix)]
        let platform = term.recv();
        #[cfg(windows)]
        let platform = close.recv();
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = platform => {}
            () = requested.notified() => {}
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let config_path = match parse_args(std::env::args().skip(1)) {
        Ok(Cli::Run { config }) => {
            config.unwrap_or_else(|| config::default_path(&config::home_dir()))
        }
        Ok(Cli::Help) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Ok(Cli::Version) => {
            println!("fleet-daemon {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("fleet-daemon: {e}\nRun fleet-daemon --help for the options.");
            return ExitCode::from(2);
        }
    };
    match run(config_path).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            if LOGGER_UP.load(Ordering::Relaxed) {
                error!(error = %e, "daemon exiting");
            }
            eprintln!("fleet-daemon: {e}");
            ExitCode::FAILURE
        }
    }
}
