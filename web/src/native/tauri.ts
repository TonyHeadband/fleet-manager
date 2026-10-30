import { invoke } from "@tauri-apps/api/core";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { Native } from "./native";

/** The bridge's commands, by the names `crates/fleet-tauri/src/bridge.rs` registers them under. */
export function tauriNative(): Native {
  return {
    version: () => invoke<string>("version"),
    hasConfig: () => invoke<boolean>("has_config"),
    startDaemon: () => invoke("start_daemon"),
    stateDir: () => invoke("state_dir"),
    firstRunDefaults: () => invoke("first_run_defaults"),
    writeInitialConfig: (root, personas) => invoke("write_initial_config", { root, personas }),
    pickFolder: (start) => invoke("pick_folder", { start }),
    installTool: (tool) => invoke("install_tool", { tool }),
    runLogin: (persona, which) => invoke("run_login", { persona, which }),
    savePat: (persona, token) => invoke("save_pat", { persona, token }),
    addMenuShortcut: () => invoke("add_menu_shortcut"),
    adoptPath: (path) => invoke("adopt_path", { path }),
    copyText: (text) => writeText(text),
    openUrl: (url) => openUrl(url),
  };
}
