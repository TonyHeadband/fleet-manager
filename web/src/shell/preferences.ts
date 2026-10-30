import { useSyncExternalStore } from "react";
import { readChoice, saveChoice, type ChoiceKey } from "./choices";

// egui's preferences (crates/fleet-client/src/config.rs's GuiSettings: theme, ui_font, mono_font, animations,
// sticky_alerts, close_choice), under gui.toml's own names, and how the window takes them.

export type Theme = "system" | "dark" | "light";
export type UiFont = "plex" | "space_grotesk" | "atkinson";
export type MonoFont = "plex" | "jetbrains" | "atkinson";
/** null is Ask: the close dialog asks each time. */
export type CloseChoice = "keep_running" | "stop_daemon" | null;

export interface Preferences {
  theme: Theme;
  uiFont: UiFont;
  monoFont: MonoFont;
  animations: boolean;
  stickyAlerts: boolean;
  closeChoice: CloseChoice;
}

export const DEFAULT_PREFERENCES: Preferences = {
  theme: "system",
  uiFont: "plex",
  monoFont: "plex",
  animations: true,
  stickyAlerts: true,
  closeChoice: null,
};

/** egui's font chips, in its order; `family` is the face `generated/theme.css` declares. */
export const UI_FONTS: { id: UiFont; label: string; family: string }[] = [
  { id: "plex", label: "IBM Plex Sans", family: "IBM Plex Sans" },
  { id: "space_grotesk", label: "Space Grotesk", family: "Space Grotesk" },
  { id: "atkinson", label: "Atkinson Hyperlegible Next", family: "Atkinson Hyperlegible Next" },
];

export const MONO_FONTS: { id: MonoFont; label: string; family: string }[] = [
  { id: "plex", label: "IBM Plex Mono", family: "IBM Plex Mono" },
  { id: "jetbrains", label: "JetBrains Mono", family: "JetBrains Mono" },
  { id: "atkinson", label: "Atkinson Hyperlegible Mono", family: "Atkinson Hyperlegible Mono" },
];

const KEY: Record<keyof Preferences, ChoiceKey> = {
  theme: "fleet.theme",
  uiFont: "fleet.uiFont",
  monoFont: "fleet.monoFont",
  animations: "fleet.animations",
  stickyAlerts: "fleet.stickyAlerts",
  closeChoice: "fleet.closeChoice",
};

/** The saved value if it is one of `allowed`, else the default: egui's lenient reader, so an unknown value from a
 * newer build or a hand edit never stops the window. */
function one<T>(saved: string | null, allowed: readonly T[], fallback: T): T {
  return allowed.find((a) => a === saved) ?? fallback;
}

const flag = (saved: string | null, fallback: boolean) => (saved === "true" ? true : saved === "false" ? false : fallback);

export function readPreferences(): Preferences {
  const d = DEFAULT_PREFERENCES;
  return {
    theme: one<Theme>(readChoice(KEY.theme), ["system", "dark", "light"], d.theme),
    uiFont: one(
      readChoice(KEY.uiFont),
      UI_FONTS.map((f) => f.id),
      d.uiFont,
    ),
    monoFont: one(
      readChoice(KEY.monoFont),
      MONO_FONTS.map((f) => f.id),
      d.monoFont,
    ),
    animations: flag(readChoice(KEY.animations), d.animations),
    stickyAlerts: flag(readChoice(KEY.stickyAlerts), d.stickyAlerts),
    closeChoice: one<CloseChoice>(readChoice(KEY.closeChoice), ["keep_running", "stop_daemon"], d.closeChoice),
  };
}

const stack = (family: string, fallback: string) => `"${family}", ${fallback}`;

/** Sets the window's look: `data-theme` (none for System, so the desktop's own switch shows at once), the two font
 * stacks, and `data-animations="off"`, which stops every animation and transition. */
export function applyPreferences(p: Preferences) {
  const root = document.documentElement;
  if (p.theme === "system") root.removeAttribute("data-theme");
  else root.dataset.theme = p.theme;
  const ui = UI_FONTS.find((f) => f.id === p.uiFont)!;
  const mono = MONO_FONTS.find((f) => f.id === p.monoFont)!;
  root.style.setProperty("--font-sans", stack(ui.family, "system-ui, sans-serif"));
  root.style.setProperty("--font-mono", stack(mono.family, "ui-monospace, monospace"));
  if (p.animations) root.removeAttribute("data-animations");
  else root.dataset.animations = "off";
}

/** Saves one preference; in the app, rejects with why `gui.toml` was not written. */
export function savePreference<K extends keyof Preferences>(key: K, value: Preferences[K]): Promise<void> {
  return saveChoice(KEY[key], typeof value === "boolean" ? String(value) : (value as string | null));
}

// One copy for the whole window, so the footer and Settings see the same values.
let current: Preferences | null = null;
const listeners = new Set<() => void>();

const snapshot = () => (current ??= readPreferences());

/** Forgets the copy, so the next reader takes what is saved now: a window starting over, or a test. */
export function resetPreferences() {
  current = null;
}

/** Applies `next` to the window and tells every reader; saving is the caller's, so a failed save still applies. */
export function setPreferences(next: Preferences) {
  current = next;
  applyPreferences(next);
  for (const l of listeners) l();
}

/** `setPreferences` for some of them, over what the window has now. */
export function mergePreferences(change: Partial<Preferences>) {
  setPreferences({ ...snapshot(), ...change });
}

function subscribe(l: () => void) {
  listeners.add(l);
  return () => void listeners.delete(l);
}

export function usePreferences(): [Preferences, (next: Preferences) => void] {
  return [useSyncExternalStore(subscribe, snapshot), setPreferences];
}
