use fleet_client::config::{self, ClientConfig};
use fleet_client::daemon_control::{connect_or_start, find_daemon_binary, stop_daemon};
use fleet_gui::shell::{EnsureFn, Root, Stopper};
use fleet_log::{LogConfig, Stderr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn config_path() -> PathBuf {
    config::config_arg(std::env::args().skip(1)).unwrap_or_else(|| config::default_path(&home()))
}

fn load() -> Result<ClientConfig, String> {
    config::load(&config_path(), &home()).map_err(|e| e.to_string())
}

/// `WxH` from the demo recorder (`FLEET_GUI_SIZE`), so a shot can be recorded at its own size.
fn parse_size(text: &str) -> Option<[f32; 2]> {
    let (w, h) = text.split_once('x')?;
    let (w, h) = (w.parse::<u32>().ok()?, h.parse::<u32>().ok()?);
    (w > 0 && h > 0).then_some([w as f32, h as f32])
}

fn window_size() -> [f32; 2] {
    const DEFAULT: [f32; 2] = [1400.0, 800.0];
    let Some(text) = std::env::var_os("FLEET_GUI_SIZE") else {
        return DEFAULT;
    };
    match text.to_str().and_then(parse_size) {
        Some(size) => size,
        None => {
            warn!("FLEET_GUI_SIZE {text:?} is not WxH; opening at 1400x800");
            DEFAULT
        }
    }
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
    let base_url = cfg
        .as_ref()
        .map(ClientConfig::base_url)
        .unwrap_or_else(|| "http://127.0.0.1:7878".into());
    let state_dir = cfg
        .map(|c| c.state_dir)
        .unwrap_or_else(|| home().join(".local/share/fleet-manager"));

    let log = fleet_log::init(LogConfig {
        file: Some(state_dir.join("gui.log")),
        stderr: Stderr::IfTerminal,
    });
    for note in notes {
        info!("{note}");
    }
    info!(version = env!("CARGO_PKG_VERSION"), "gui started");

    let ensure: EnsureFn = Arc::new(|| {
        let cfg = load()?;
        connect_or_start(&cfg, find_daemon_binary, Duration::from_secs(10))
            .map_err(|e| e.to_string())
    });
    let stop: Stopper = Arc::new(|| stop_daemon(&load()?.state_dir).map_err(|e| e.to_string()));

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size(window_size())
            .with_title(fleet_gui::theme::APP_NAME)
            .with_app_id("fleet-manager")
            // Without it the title bar and taskbar show a generic icon unless an installed .desktop file matches the
            // app id, and X11 window managers never look at that file.
            .with_icon(Arc::new(
                eframe::icon_data::from_png_bytes(fleet_gui::icon::png(true))
                    .expect("bundled icon is a valid PNG"),
            )),
        ..Default::default()
    };
    eframe::run_native(
        "fleet-manager",
        options,
        Box::new(move |cc| {
            fleet_gui::theme::apply(&cc.egui_ctx);
            // Before the first frame, so a saved Light/Dark choice never flashes the system theme first.
            let saved = config::GuiSettings::load(&config::gui_settings_path(&home()));
            cc.egui_ctx
                .set_theme(fleet_gui::theme::preference(saved.theme));
            let mut root = Root::new(
                cc.egui_ctx.clone(),
                base_url,
                config::gui_settings_path(&home()),
                state_dir,
                config_path(),
                home(),
                ensure,
                stop,
                log.clone(),
            );
            // Set only by the demo recorder (`scripts/demo.sh`), which cannot click into the window.
            if let Some(path) = std::env::var_os("FLEET_GUI_CUES") {
                // A relaunch after a `quit` cue starts past the cues its predecessor applied.
                let from = std::env::var("FLEET_GUI_CUES_FROM")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                root = root.with_cues(PathBuf::from(path), from);
            }
            Ok(Box::new(root))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::parse_size;

    #[test]
    fn size_parses_wxh_only() {
        assert_eq!(parse_size("1280x720"), Some([1280.0, 720.0]));
        for bad in ["", "1280", "1280x", "x720", "0x720", "axb", "1280X720"] {
            assert_eq!(parse_size(bad), None, "{bad}");
        }
    }
}
