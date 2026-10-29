#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
use fleet_client::config::{self, ClientConfig};
use fleet_client::daemon_control::{connect_or_start, find_daemon_binary, stop_daemon};
use fleet_gui::shell::{EnsureFn, Root, Stopper};
use fleet_log::{LogConfig, Stderr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

fn home() -> PathBuf {
    fleet_fs::dirs::home_dir()
}

fn config_path() -> PathBuf {
    config::config_arg(std::env::args().skip(1)).unwrap_or_else(|| config::default_path(&home()))
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
    // fleet-board only ever ran on Linux: nothing to move on Windows.
    #[cfg(unix)]
    let notes = fleet_fs::migrate::migrate_legacy_dirs(&home());
    #[cfg(not(unix))]
    let notes: Vec<String> = Vec::new();
    // Read once for the address and state directory; a broken config is reported on the startup screen, where
    // Retry re-reads it, so the window still opens.
    let cfg = load().ok();
    let base_url = cfg
        .as_ref()
        .map(ClientConfig::base_url)
        .unwrap_or_else(|| "http://127.0.0.1:7878".into());
    let state_dir = cfg
        .map(|c| c.state_dir)
        .unwrap_or_else(|| fleet_fs::dirs::data_dir(&home()));

    let log_path = state_dir.join("gui.log");
    let log = fleet_log::init(LogConfig {
        file: Some(log_path.clone()),
        stderr: Stderr::IfTerminal,
    });
    // The release exe has no console, so a panic would otherwise vanish.
    fleet_log::install_panic_hook();
    #[cfg(all(windows, not(debug_assertions)))]
    {
        let logged = std::panic::take_hook();
        let path = log_path.clone();
        std::panic::set_hook(Box::new(move |info| {
            logged(info);
            if std::thread::current().name() == Some("main") {
                message_box(&fleet_log::panic_text(info), &path);
            }
        }));
    }
    for note in notes {
        info!("{note}");
    }
    info!(version = env!("CARGO_PKG_VERSION"), "gui started");

    let ensure: EnsureFn = Arc::new(|| {
        let cfg = load()?;
        connect_or_start(&cfg, find_daemon_binary, Duration::from_secs(10))
            .map_err(|e| e.to_string())
    });
    let stop: Stopper = Arc::new(|| {
        let cfg = load()?;
        stop_daemon(&cfg.state_dir, &cfg.base_url()).map_err(|e| e.to_string())
    });

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 800.0])
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
    let result = eframe::run_native(
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
            if let Ok(exe) = std::env::current_exe() {
                root = root.with_shortcuts(fleet_client::shortcuts::places(), exe);
            }
            // Set only by the demo recorder (`scripts/demo.sh`), which cannot click into the window.
            // The recording stays free of a new-version notice.
            if let Some(path) = std::env::var_os("FLEET_GUI_CUES") {
                root = root.with_cues(PathBuf::from(path));
            } else {
                root = root.with_releases(Arc::new(|| {
                    fleet_client::update::latest(&fleet_client::update::releases_url())
                }));
            }
            Ok(Box::new(root))
        }),
    );
    if let Err(e) = &result {
        tracing::error!("could not open the window: {e}");
        #[cfg(all(windows, not(debug_assertions)))]
        message_box(
            &format!("fleet-manager could not open its window: {e}"),
            &log_path,
        );
    }
    result
}

/// The release exe has no console: a fatal error is shown in a message box that names the log.
#[cfg(all(windows, not(debug_assertions)))]
fn message_box(what: &str, log_path: &std::path::Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    let wide = |s: &std::ffi::OsStr| s.encode_wide().chain([0]).collect::<Vec<u16>>();
    let text = format!(
        "{what}

Details are in {}",
        log_path.display()
    );
    // SAFETY: both strings are NUL-terminated and outlive the call; no owner window.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            wide(text.as_ref()).as_ptr(),
            wide("fleet-manager".as_ref()).as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}
