import { useCallback, useEffect, useRef, useState } from "react";
import type { Effort } from "../api/generated/Effort";
import type { PermissionMode } from "../api/generated/PermissionMode";
import type { DaemonSettings, LimitsDto, Native, PrDto, SettingsChange } from "../native/native";
import type { RestartFor } from "./restart";

// The daemon's settings as General types them (egui's root_text, personas_text, limits_text, pr_typed and
// agent_defaults_typed in crates/fleet-gui/src/app.rs), and why each Save cannot run now.

/** The Pull requests rows as typed: the labels stay one comma-separated line until saved (egui's `PrDraft`). */
export interface PrTyped extends Omit<PrDto, "labels"> {
  labels: string;
}

export interface Typed {
  tasksRoot: string;
  personasDir: string;
  /** In `LIMITS` order. */
  limits: string[];
  pr: PrTyped;
  mode: PermissionMode;
  effort: Effort;
}

export type SaveFor = Exclude<RestartFor, "restart">;

const LIMIT_KEYS = [
  "maxBreakdownVersions",
  "maxSteps",
  "maxParallelSpecialists",
  "maxPrAttempts",
  "maxConcurrentTasks",
  "maxActiveProducts",
] as const;

const U32_MAX = 4294967295;

/** egui's `config::parse_limit`: a u32 above zero, as Rust parses one (a leading `+` allowed). */
function limit(text: string) {
  const t = text.trim();
  const n = Number(t);
  return /^\+?\d+$/.test(t) && n > 0 && n <= U32_MAX ? n : null;
}

/** egui's `typed_limits`: each a whole number above zero; the two task limits may be empty for no cap. */
export function typedLimits(texts: readonly string[]): LimitsDto | null {
  const out: Record<string, number | null> = {};
  for (const [i, key] of LIMIT_KEYS.entries()) {
    const text = texts[i] ?? "";
    const optional = i >= 4 && text.trim() === "";
    const n = optional ? null : limit(text);
    if (n === null && !optional) return null;
    out[key] = n;
  }
  return out as unknown as LimitsDto;
}

/** egui's `parse_labels`. */
export const prLabels = (text: string) =>
  text
    .split(",")
    .map((l) => l.trim())
    .filter((l) => l !== "");

/** egui's `config::expand_home` compared as paths: `~` is home, and a trailing `/` names the same folder. */
export function sameFolder(typed: string, saved: string, home: string) {
  const norm = (t: string) => {
    const s = t.trim();
    const full = s === "~" ? home : s.startsWith("~/") ? home + s.slice(1) : s;
    return full.length > 1 ? full.replace(/\/+$/, "") : full;
  };
  return norm(typed) === norm(saved);
}

const sameLimits = (a: LimitsDto, b: LimitsDto) => LIMIT_KEYS.every((k) => a[k] === b[k]);

/** The PR settings a Save writes: the attribution trimmed, the labels parsed (egui's `PrDraft::to_settings`). */
export const prOf = (t: PrTyped): PrDto => ({ ...t, attribution: t.attribution.trim(), labels: prLabels(t.labels) });

const samePr = (a: PrDto, b: PrDto) =>
  a.style === b.style &&
  a.attribution === b.attribution &&
  a.draft === b.draft &&
  a.gateBuild === b.gateBuild &&
  a.labels.join("\n") === b.labels.join("\n");

function typedOf(s: DaemonSettings): Typed {
  return {
    // The folders the daemon runs with: one the wizard wrote since waits for the restart (egui's `root_text`).
    tasksRoot: s.runningTasksRoot,
    personasDir: s.runningPersonasDir,
    limits: LIMIT_KEYS.map((k) => (s.limits[k] === null ? "" : String(s.limits[k]))),
    pr: { ...s.pr, labels: s.pr.labels.join(", ") },
    mode: s.defaultMode,
    effort: s.defaultEffort,
  };
}

/** What each Save writes. Limits that do not parse write nothing: their Save is blocked. */
export function changeOf(kind: SaveFor, t: Typed): SettingsChange | null {
  switch (kind) {
    case "tasksRoot":
      return { kind, text: t.tasksRoot };
    case "personasDir":
      return { kind, text: t.personasDir };
    case "limits": {
      const l = typedLimits(t.limits);
      return l && { kind, ...l };
    }
    case "pr":
      return { kind, ...prOf(t.pr) };
    case "agentDefaults":
      return { kind, mode: t.mode, effort: t.effort };
  }
}

/** egui's `save_blocker`, `limits_blocker`, `pr_blocker` and `agent_defaults_blocker`: what is typed being the
 * current setting first, then the restart's own blocker. */
export function saveBlockers(saved: DaemonSettings, t: Typed, restartBlocker: string | null): Record<SaveFor, string | null> {
  const limits = typedLimits(t.limits);
  return {
    tasksRoot: sameFolder(t.tasksRoot, saved.runningTasksRoot, saved.home) ? "This is the current tasks folder" : restartBlocker,
    personasDir: sameFolder(t.personasDir, saved.runningPersonasDir, saved.home) ? "This is the current personas folder" : restartBlocker,
    limits:
      limits === null
        ? "Each limit is a whole number above zero; the task limits may be empty for no cap"
        : sameLimits(limits, saved.limits)
          ? "These are the current limits"
          : restartBlocker,
    pr: samePr(prOf(t.pr), saved.pr) ? "These are the current pull request settings" : restartBlocker,
    agentDefaults: t.mode === saved.defaultMode && t.effort === saved.defaultEffort ? "These are the current defaults" : restartBlocker,
  };
}

/** egui's `pending_folders_text`: the folders `config.toml` has that the daemon does not run with yet. */
export function pendingFolders(saved: DaemonSettings): string | null {
  const parts = [
    ["Tasks folder", saved.tasksRoot, saved.runningTasksRoot],
    ["Personas folder", saved.personasDir, saved.runningPersonasDir],
  ]
    .filter(([, configured, running]) => !sameFolder(configured!, running!, saved.home))
    .map(([name, configured]) => `${name} ${configured}`);
  return parts.length > 0 ? parts.join(", ") : null;
}

const FOLDERS = [
  ["tasksRoot", "runningTasksRoot"],
  ["personasDir", "runningPersonasDir"],
] as const;

/** The settings `config.toml` has and the fields as typed; null in a browser and until read. */
export function useDaemonSettings(native: Native | null) {
  const [saved, setSaved] = useState<DaemonSettings | null>(null);
  const [typed, setTyped] = useState<Typed | null>(null);

  useEffect(() => {
    if (!native) return;
    let live = true;
    native.readDaemonSettings().then(
      (s) => {
        if (!live) return;
        setSaved(s);
        setTyped(typedOf(s));
      },
      () => {},
    );
    return () => {
      live = false;
    };
  }, [native]);

  const set = useCallback((patch: Partial<Typed>) => setTyped((t) => t && { ...t, ...patch }), []);
  // The settings the fields last followed, so a restart can tell a field still showing the old folder.
  const last = useRef(saved);
  last.current = saved;

  /** A Save's write landed or failed: the config is read again, which says what was written (a failed restart keeps
   * it). A folder written replaces its field, `~`-shortened, only when it was not edited meanwhile (egui's
   * `RootSaved`); when the daemon now runs with another folder, a field still showing the old one, however spelled,
   * follows (egui's `apply_pending_folders`). Other fields keep what is typed. */
  const settled = useCallback(
    (change: SettingsChange) => {
      const before = last.current;
      native?.readDaemonSettings().then(
        (s) => {
          setSaved(s);
          setTyped((t) => {
            if (!t) return t;
            const next = { ...t };
            for (const [key, running] of FOLDERS) {
              if (change.kind === key && t[key] === change.text) {
                if (sameFolder(t[key], s[key], s.home)) next[key] = s[key];
              } else if (before && !sameFolder(before[running], s[running], s.home) && sameFolder(t[key], before[running], s.home))
                next[key] = s[running];
            }
            return next;
          });
        },
        () => {},
      );
    },
    [native],
  );

  /** The relaunched wizard wrote folders: they wait for the restart, so only the saved copy changes. Resolves once it
   * has. */
  const wizardSaved = useCallback(
    (): Promise<void> => (native ? native.readDaemonSettings().then(setSaved, () => {}) : Promise.resolve()),
    [native],
  );

  return { saved, typed, set, settled, wizardSaved };
}

export type DaemonSettingsApi = ReturnType<typeof useDaemonSettings>;
