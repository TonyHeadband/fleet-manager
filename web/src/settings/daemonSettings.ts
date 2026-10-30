import { useCallback, useEffect, useState } from "react";
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

/** egui's `config::parse_limit`: a whole number above zero. */
const limit = (text: string) => (/^\d+$/.test(text.trim()) && Number(text) > 0 ? Number(text) : null);

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
    tasksRoot: s.tasksRoot,
    personasDir: s.personasDir,
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
    tasksRoot: sameFolder(t.tasksRoot, saved.tasksRoot, saved.home) ? "This is the current tasks folder" : restartBlocker,
    personasDir: sameFolder(t.personasDir, saved.personasDir, saved.home) ? "This is the current personas folder" : restartBlocker,
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

  /** A Save's write landed or failed: the config is read again (a failed restart keeps what was written). A saved
   * folder replaces its field only when it was not edited meanwhile (egui's `RootSaved`); other fields keep what is
   * typed. */
  const settled = useCallback(
    (change: SettingsChange, ok: boolean) => {
      native?.readDaemonSettings().then(
        (s) => {
          setSaved(s);
          if (ok && (change.kind === "tasksRoot" || change.kind === "personasDir")) {
            const key = change.kind;
            setTyped((t) => (t && t[key] === change.text ? { ...t, [key]: s[key] } : t));
          }
        },
        () => {},
      );
    },
    [native],
  );

  return { saved, typed, set, settled };
}

export type DaemonSettingsApi = ReturnType<typeof useDaemonSettings>;
