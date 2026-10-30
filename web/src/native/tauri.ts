import { invoke } from "@tauri-apps/api/core";
import type { Native } from "./native";

/** The bridge's commands, by the names `crates/fleet-tauri/src/bridge.rs` registers them under. */
export function tauriNative(): Native {
  return {
    daemonUrl: () => invoke<string>("daemon_url"),
    version: () => invoke<string>("version"),
    hasConfig: () => invoke<boolean>("has_config"),
  };
}
