import { useCallback, useRef, useState } from "react";
import type { FleetClient } from "../api/client";
import type { AgentConfig } from "../api/generated/AgentConfig";
import type { AgentSettings } from "../api/generated/AgentSettings";

// A port of crates/fleet-gui/src/views/management/mod.rs's ConfigState and of the agent-config half of
// app/setup.rs: the instructions and settings every agent gets, each with its own draft, error and Save.

export type ConfigPart = "instructions" | "settings";

/** The settings as edited: rows in order, blank ones included, a variable as a name and value. */
export interface SettingsDraft {
  allow: string[];
  deny: string[];
  env: [string, string][];
}

export interface ConfigState {
  saved: AgentConfig;
  instructions: string;
  settings: SettingsDraft;
  load: "not-loaded" | "loading" | "loaded" | "failed";
  /** The first answer came, a config or an error: the editors show. */
  answered: boolean;
  loadError: string | null;
  instructionsError: string | null;
  settingsError: string | null;
  saving: ConfigPart | null;
}

const EMPTY: AgentConfig = { instructions: "", settings: { allow: [], deny: [], env: {} }, settings_error: null };

export const initialConfig: ConfigState = {
  saved: EMPTY,
  instructions: "",
  settings: { allow: [], deny: [], env: [] },
  load: "not-loaded",
  answered: false,
  loadError: null,
  instructionsError: null,
  settingsError: null,
  saving: null,
};

export const draftOf = (s: AgentSettings): SettingsDraft => ({ allow: [...s.allow], deny: [...s.deny], env: Object.entries(s.env) });

const same = (a: SettingsDraft, b: SettingsDraft) => JSON.stringify(a) === JSON.stringify(b);
export const instructionsDirty = (st: ConfigState) => st.instructions !== st.saved.instructions;
export const settingsDirty = (st: ConfigState) => !same(st.settings, draftOf(st.saved.settings));

/** egui's `to_settings`: blank rules dropped, variables named once each. */
export function toSettings(d: SettingsDraft): AgentSettings | string {
  const rules = (rows: string[]) => rows.map((r) => r.trim()).filter((r) => r !== "");
  const env: Record<string, string> = {};
  for (const [name, value] of d.env) {
    const key = name.trim();
    if (key === "") {
      if (value === "") continue;
      return "An environment variable needs a name.";
    }
    if (key in env) return `${key} is set twice.`;
    env[key] = value;
  }
  return { allow: rules(d.allow), deny: rules(d.deny), env };
}

/** The daemon's config arrived: a part being edited keeps its draft, a clean one follows the daemon. */
export function loaded(st: ConfigState, cfg: AgentConfig): ConfigState {
  return {
    ...follow(st, cfg, instructionsDirty(st), settingsDirty(st)),
    load: "loaded",
    answered: true,
    loadError: null,
  };
}

/** A save landed: the part saved takes the daemon's copy as its draft; the other keeps its draft if edited. */
export function saved(st: ConfigState, cfg: AgentConfig): ConfigState {
  const part = st.saving;
  return {
    ...follow(st, cfg, part !== "instructions" && instructionsDirty(st), part !== "settings" && settingsDirty(st)),
    saving: null,
  };
}

function follow(st: ConfigState, cfg: AgentConfig, keepInstructions: boolean, keepSettings: boolean): ConfigState {
  return {
    ...st,
    saved: cfg,
    instructions: keepInstructions ? st.instructions : cfg.instructions,
    instructionsError: keepInstructions ? st.instructionsError : null,
    settings: keepSettings ? st.settings : draftOf(cfg.settings),
    settingsError: keepSettings ? st.settingsError : null,
  };
}

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** The config and its calls, kept above Management so drafts outlive the tab and the view. */
export function useAgentConfig(client: FleetClient) {
  const [st, setSt] = useState<ConfigState>(initialConfig);
  // Read synchronously, so a second load or save asked for before a render is refused, as egui's fields are.
  const now = useRef(st);
  now.current = st;
  const update = useCallback((f: (s: ConfigState) => ConfigState) => {
    now.current = f(now.current);
    setSt(now.current);
  }, []);

  /** Loaded when first opened, not at start; a failed load is asked again on each way back in. */
  const load = useCallback(() => {
    if (now.current.load === "loaded" || now.current.load === "loading") return;
    update((s) => ({ ...s, load: "loading" }));
    client.agentConfig().then(
      (cfg) => update((s) => loaded(s, cfg)),
      // The editors stay usable and empty: saving a valid config is how a broken file is repaired.
      (e: unknown) => update((s) => ({ ...s, load: "failed", answered: true, loadError: message(e) })),
    );
  }, [client, update]);

  const save = (part: ConfigPart, body: { instructions?: string; settings?: AgentSettings }) => {
    update((s) => ({ ...s, saving: part }));
    client.saveAgentConfig(body).then(
      (cfg) => update((s) => saved(s, cfg)),
      (e: unknown) =>
        update((s) => {
          // No save in flight: an answer to one that is gone.
          if (s.saving === null) return s;
          return { ...s, saving: null, [s.saving === "settings" ? "settingsError" : "instructionsError"]: message(e) };
        }),
    );
  };

  return {
    st,
    load,
    setInstructions: (text: string) => update((s) => ({ ...s, instructions: text })),
    setSettings: (settings: SettingsDraft) => update((s) => ({ ...s, settings })),
    saveInstructions: () => {
      update((s) => ({ ...s, instructionsError: null }));
      save("instructions", { instructions: now.current.instructions });
    },
    saveSettings: () => {
      const settings = toSettings(now.current.settings);
      if (typeof settings === "string") return update((s) => ({ ...s, settingsError: settings }));
      update((s) => ({ ...s, settingsError: null }));
      save("settings", { settings });
    },
    revertInstructions: () => update((s) => ({ ...s, instructions: s.saved.instructions, instructionsError: null })),
    revertSettings: () => update((s) => ({ ...s, settings: draftOf(s.saved.settings), settingsError: null })),
  };
}

export type AgentConfigApi = ReturnType<typeof useAgentConfig>;
