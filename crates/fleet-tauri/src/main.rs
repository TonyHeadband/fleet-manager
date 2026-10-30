//! Fleet Manager's native app: the web UI in a Tauri window. The window opens on the web UI's boot view at once;
//! starting or finding the daemon, the wizard and the close flow go through the bridge (`bridge.rs`).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod bridge;

use bridge::Paths;

fn main() {
    if std::env::args().any(|a| a == "--version") {
        println!("fleet-gui-tauri {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let paths = Paths::from_args(std::env::args().skip(1));
    let state_dir = paths.load().state_dir;
    let _log = fleet_log::init(fleet_log::LogConfig {
        file: Some(state_dir.join("gui.log")),
        stderr: fleet_log::Stderr::IfTerminal,
    });
    fleet_log::install_panic_hook();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "tauri gui started");

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(paths)
        .manage(bridge::Owner::default())
        .invoke_handler(tauri::generate_handler![
            bridge::version,
            bridge::has_config,
            bridge::start_daemon,
            bridge::state_dir
        ])
        .run(tauri::generate_context!())
        .expect("run the Tauri app");
}
