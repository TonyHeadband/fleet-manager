import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { CloseEvent, LogEntry, Native } from "./native";

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
    shortcuts: () => invoke("shortcuts_state"),
    changeShortcuts: (action, menu, desktop) => invoke("change_shortcuts", { action, menu, desktop }),
    adoptPath: (path) => invoke("adopt_path", { path }),
    copyText: (text) => writeText(text),
    openUrl: (url) => openUrl(url),
    onClose: (handler) => {
      const stop = listen<CloseEvent>("close", (e) => handler(e.payload));
      return () => void stop.then((unlisten) => unlisten());
    },
    answerClose: (answer, remember) => invoke("answer_close", { answer, remember }),
    setCloseArmed: (armed) => invoke("set_close_armed", { armed }),
    openTerminal: (agent) => invoke("open_terminal", { agent }),
    readDaemonSettings: () => invoke("read_daemon_settings"),
    saveAndRestart: (change) => invoke("save_and_restart", { change }),
    saveSettings: (change) => invoke("save_settings", { change }),
    daemonInfo: () => invoke("daemon_info"),
    logOpen: () => invoke("log_open"),
    logPoll: () => invoke("log_poll"),
    logLive: () => invoke("log_live"),
    onLogEntry: (handler) => {
      const stop = listen<LogEntry>("log-entry", (e) => handler(e.payload));
      return () => void stop.then((unlisten) => unlisten());
    },
    logNotice: (kind, text) => invoke("log_notice", { kind, text }),
    logFileError: () => invoke("log_file_error"),
    checkUpdate: () => invoke("check_update"),
    skipVersion: (version) => invoke("skip_version", { version }),
    readChoices: () => invoke("read_choices"),
    writeChoice: (key, value) => invoke("write_choice", { key, value }),
  };
}
