use fleet_client::config::{self, ClientConfig};
use fleet_client::daemon_control::{connect_or_start, find_daemon_binary, stop_daemon};
use fleet_gui::shell::{EnsureFn, Root, Stopper};
use fleet_log::{LogConfig, Stderr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

/// Rendered from `packaging/fleet-manager.svg` at 256 px; re-render it when the SVG changes.
const ICON: &[u8] = include_bytes!("../assets/icon.png");

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
    // Before the logger: it opens `gui.log` in the (possibly just migrated) state directory.
    let notes = fleet_fs::migrate::migrate_legacy_dirs(&home());
    // Read once for the address and state directory; a broken config is reported on the startup screen, where
    // Retry re-reads it, so the window still opens.
    let cfg = load().ok();
    let base_url = cfg.as_ref().map(ClientConfig::base_url).unwrap_or_else(|| "http://127.0.0.1:7878".into());
    let state_dir = cfg.map(|c| c.state_dir).unwrap_or_else(|| home().join(".local/share/fleet-manager"));

    let log = fleet_log::init(LogConfig { file: Some(state_dir.join("gui.log")), stderr: Stderr::IfTerminal });
    for note in notes {
        info!("{note}");
    }
    info!(version = env!("CARGO_PKG_VERSION"), "gui started");

    let ensure: EnsureFn = Arc::new(|| {
        let cfg = load()?;
        connect_or_start(&cfg, find_daemon_binary, Duration::from_secs(10)).map_err(|e| e.to_string())
    });
    let stop: Stopper = Arc::new(|| stop_daemon(&load()?.state_dir).map_err(|e| e.to_string()));

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 800.0])
            .with_title(fleet_gui::theme::APP_NAME)
            .with_app_id("fleet-manager")
            // Without it the title bar and taskbar show a generic icon unless an installed .desktop file matches the
            // app id, and X11 window managers never look at that file.
            .with_icon(Arc::new(eframe::icon_data::from_png_bytes(ICON).expect("bundled icon is a valid PNG"))),
        ..Default::default()
    };
    eframe::run_native(
        "fleet-manager",
        options,
        Box::new(move |cc| {
            fleet_gui::theme::apply(&cc.egui_ctx);
            // Before the first frame, so a saved Light/Dark choice never flashes the system theme first.
            let saved = config::GuiSettings::load(&config::gui_settings_path(&home()));
            cc.egui_ctx.set_theme(fleet_gui::theme::preference(saved.theme));
            Ok(Box::new(Root::new(
                cc.egui_ctx.clone(),
                base_url,
                config::gui_settings_path(&home()),
                state_dir,
                config_path(),
                home(),
                ensure,
                stop,
                log.clone(),
            )))
        }),
    )
}
