import { invoke } from "@tauri-apps/api/core";
import type { Native } from "./native";

/** The bridge's commands, by the names `crates/fleet-tauri/src/bridge.rs` registers them under. */
export function tauriNative(): Native {
  return {
    version: () => invoke<string>("version"),
    hasConfig: () => invoke<boolean>("has_config"),
    startDaemon: () => invoke<{ weStarted: boolean; url: string }>("start_daemon"),
    stateDir: () => invoke<string>("state_dir"),
  };
}
