use fleet_client::config::{self, ClientConfig};
use fleet_client::daemon_control::{ensure_daemon, find_daemon_binary, stop_daemon};
use fleet_gui::shell::{EnsureFn, Root, Stopper};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

fn config_path() -> PathBuf {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--config" {
            if let Some(p) = args.next() {
                return PathBuf::from(p);
            }
        }
    }
    config::default_path(&home())
}

fn load() -> Result<ClientConfig, String> {
    config::load(&config_path(), &home()).map_err(|e| e.to_string())
}

fn main() -> eframe::Result {
    if std::env::args().any(|a| a == "--version") {
        println!("fleet-gui {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    // Read once for the address and state directory; a broken config is reported on the startup screen, where
    // Retry re-reads it, so the window still opens.
    let cfg = load().ok();
    let base_url = cfg.as_ref().map(ClientConfig::base_url).unwrap_or_else(|| "http://127.0.0.1:7878".into());
    let state_dir = cfg.map(|c| c.state_dir).unwrap_or_else(|| home().join(".local/share/fleet-board"));

    let ensure: EnsureFn = Arc::new(|| {
        let cfg = load()?;
        let bin = find_daemon_binary().ok_or("fleet-daemon was not found next to fleet-gui or on PATH")?;
        ensure_daemon(&cfg, &bin, Duration::from_secs(10)).map_err(|e| e.to_string())
    });
    let stop: Stopper = Arc::new(|| stop_daemon(&load()?.state_dir).map_err(|e| e.to_string()));

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 800.0])
            .with_title("fleet-board")
            .with_app_id("fleet-board"),
        ..Default::default()
    };
    eframe::run_native(
        "fleet-board",
        options,
        Box::new(move |cc| {
            fleet_gui::theme::apply(&cc.egui_ctx);
            Ok(Box::new(Root::new(cc.egui_ctx.clone(), base_url, config::gui_settings_path(&home()), state_dir, ensure, stop)))
        }),
    )
}
