import type { DefInfo } from "../api/generated/DefInfo";
import type { EffectiveSettings } from "../api/generated/EffectiveSettings";
import type { Product } from "../api/generated/Product";
import type { ProductOverride } from "../api/generated/ProductOverride";
import { usable } from "./definitions";

// A port of the override rows' rules in crates/fleet-gui/src/views/management/products.rs.

/** An override row as being edited; `error` is the daemon's, from a read. */
export interface OverrideDraft {
  section: string;
  key: string;
  value: string;
  error: string | null;
}

export const SECTIONS = ["models", "modes", "efforts", "limits", "pr"];
const LIMIT_KEYS = ["max_breakdown_versions", "max_steps", "max_parallel_specialists", "max_pr_attempts", "max_concurrent_tasks"];
const PR_KEYS = ["style", "attribution", "draft", "labels", "gate_build"];
const MODE_VALUES = ["auto", "acceptEdits", "manual", "dontAsk"];
const EFFORT_VALUES = ["low", "medium", "high", "xhigh", "max"];

export const newOverride = (): OverrideDraft => ({ section: SECTIONS[0]!, key: "", value: "", error: null });

export const drafts = (p: Product): OverrideDraft[] =>
  p.overrides.map((o) => ({ section: o.section, key: o.key, value: o.value, error: o.error ?? null }));

/** Order-sensitive, on what is sent: a row's error is not part of it. */
export const sameRows = (a: OverrideDraft[], b: OverrideDraft[]) =>
  a.length === b.length && a.every((r, i) => r.section === b[i]!.section && r.key === b[i]!.key && r.value === b[i]!.value);

export const ready = (r: OverrideDraft) => r.key.trim() !== "" && r.value.trim() !== "";

export const toOverride = (r: OverrideDraft): ProductOverride => ({ section: r.section, key: r.key.trim(), value: r.value.trim() });

/**
 * A row after `change`: the old key means nothing in another section, and in `pr` a style name means nothing for
 * `draft`, and the reverse. Any change clears the daemon's error.
 */
export function editRow(row: OverrideDraft, change: Partial<Pick<OverrideDraft, "section" | "key" | "value">>): OverrideDraft {
  const next = { ...row, ...change };
  if (next.section !== row.section) next.key = "";
  if (next.section === "pr" && next.key !== row.key) next.value = "";
  const same = next.section === row.section && next.key === row.key && next.value === row.value;
  return same ? next : { ...next, error: null };
}

/** The keys a section picks from: the usable orchestrators then specialists for models, modes and efforts. */
export function keyOptions(section: string, defs: DefInfo[] | null): string[] {
  if (section === "limits") return LIMIT_KEYS;
  if (section === "pr") return PR_KEYS;
  return defs ? [...usable(defs, "orchestrator"), ...usable(defs, "specialist")] : [];
}

/** The values a key picks from, when it has a fixed set; null for free text. */
export function valueOptions(section: string, key: string, defs: DefInfo[] | null): string[] | null {
  if (section === "modes") return MODE_VALUES;
  if (section === "efforts") return EFFORT_VALUES;
  if (section !== "pr") return null;
  if (key === "style") return defs ? usable(defs, "pr_style") : [];
  if (key === "draft") return ["true", "false"];
  if (key === "gate_build") return ["shared", "clean"];
  return null;
}

/** The free-text value's hint. */
export const valueHint = (section: string) => (section === "limits" ? "number" : section === "pr" ? "text" : "model");

/** What the row's setting is without the override, and where that came from; null until the daemon has said. */
export function replaces(effective: EffectiveSettings | null, row: OverrideDraft): string | null {
  const v = effective?.values.find((e) => e.section === row.section && e.key === row.key);
  if (!v) return null;
  if (v.value !== null) return `replaces ${v.value} (${v.source})`;
  return row.section === "limits" ? "replaces no cap" : "replaces persona default";
}
