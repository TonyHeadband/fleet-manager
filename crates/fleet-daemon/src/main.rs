use fleet_daemon::agents::{AgentService, ServiceOptions};
use fleet_daemon::config::{self, Config};
use fleet_daemon::http::{bind_loopback, router, AppState};
use fleet_daemon::store::Store;
use fleet_daemon::tmux::Tmux;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

const RECONCILE_EVERY: Duration = Duration::from_secs(5);
/// A dedicated tmux server keeps agent sessions apart from the user's own tmux.
/// Attach with `tmux -L fleet-board attach -t <session>`.
const DEFAULT_TMUX_SOCKET: &str = "fleet-board";

fn config_path() -> PathBuf {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--config" {
            if let Some(p) = args.next() {
                return PathBuf::from(p);
            }
        }
    }
    config::default_path(&config::home_dir())
}

async fn run() -> Result<(), String> {
    let cfg: Config = config::load(&config_path()).map_err(|e| e.to_string())?;
    let tmux = Tmux::new(Some(std::env::var("FLEET_TMUX_SOCKET").unwrap_or_else(|_| DEFAULT_TMUX_SOCKET.into())));
    tmux.ensure_supported().await.map_err(|e| e.to_string())?;

    // Bind first: the hook URLs handed to agents must carry the port actually in use.
    let listener = bind_loopback(cfg.listen).await.map_err(|e| e.to_string())?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;

    std::fs::create_dir_all(&cfg.state_dir).map_err(|e| format!("cannot create {}: {e}", cfg.state_dir.display()))?;
    let store = Arc::new(Store::open(&cfg.state_dir.join("fleet.db")).await.map_err(|e| e.to_string())?);
    let mut opts = ServiceOptions::new(&format!("http://{addr}"));
    opts.claude = cfg.claude_command.clone();
    let svc = Arc::new(AgentService::new(cfg, store.clone(), tmux.clone(), opts));
    svc.reconcile().await.map_err(|e| e.to_string())?;
    // Hooks sent while the daemon was down are lost; say so for agents whose state may have moved on.
    svc.mark_unverified().await.map_err(|e| e.to_string())?;

    let ticker = svc.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(RECONCILE_EVERY).await;
            if let Err(e) = ticker.reconcile().await {
                eprintln!("reconcile failed: {e}");
            }
        }
    });

    println!("fleet-board listening on http://{addr}");
    axum::serve(listener, router(AppState::new(svc, store, tmux, addr.port())))
        .with_graceful_shutdown(async { let _ = tokio::signal::ctrl_c().await; })
        .await
        .map_err(|e| e.to_string())
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("fleet-daemon: {e}");
            ExitCode::FAILURE
        }
    }
}
