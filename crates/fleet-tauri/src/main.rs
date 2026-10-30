//! Fleet Manager's native app: the web UI in a Tauri window. The window opens on the web UI's boot view at once;
//! starting or finding the daemon, the wizard and the close flow go through the bridge (`bridge.rs`).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod bridge;
mod close;
#[cfg(target_os = "linux")]
mod webkit;

use bridge::Paths;
use tauri::Manager;

fn main() {
    if std::env::args().any(|a| a == "--version") {
        println!("fleet-gui-tauri {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    #[cfg(target_os = "linux")]
    let webkit_workaround = webkit::apply();
    let paths = Paths::from_args(std::env::args().skip(1));
    let state_dir = paths.load().state_dir;
    let _log = fleet_log::init(fleet_log::LogConfig {
        file: Some(state_dir.join("gui.log")),
        stderr: fleet_log::Stderr::IfTerminal,
    });
    fleet_log::install_panic_hook();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "tauri gui started");
    #[cfg(target_os = "linux")]
    if webkit_workaround {
        tracing::info!(
            "NVIDIA under Wayland: WebKitGTK's DMA-BUF renderer is off ({}=1)",
            webkit::VAR
        );
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(paths)
        .manage(bridge::Owner::default())
        .manage(close::CloseState::default())
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if !close::requested(window.app_handle()) {
                    api.prevent_close();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            bridge::version,
            bridge::has_config,
            bridge::start_daemon,
            bridge::state_dir,
            bridge::first_run_defaults,
            bridge::write_initial_config,
            bridge::pick_folder,
            bridge::install_tool,
            bridge::adopt_path,
            bridge::run_login,
            bridge::save_pat,
            bridge::add_menu_shortcut,
            bridge::open_terminal,
            bridge::restart_daemon,
            bridge::check_update,
            bridge::skip_version,
            bridge::read_choices,
            bridge::write_choice,
            close::answer_close,
            close::set_close_armed
        ])
        .run(tauri::generate_context!())
        .expect("run the Tauri app");
}
