import { useEffect, useReducer, useRef } from "react";
import type { ActionDef } from "../api/generated/ActionDef";
import type { Agent } from "../api/generated/Agent";
import type { FieldDef } from "../api/generated/FieldDef";
import type { Tone } from "../api/generated/Tone";
import { evalCondition, fieldValue } from "../api/schema";
import { card } from "../generated/layout";
import type { IconName } from "../ui/icons";

// Lite's card rules (crates/fleet-gui/src/components/card.rs), ported so both boards place and pick the same.

/** Where a schema field is drawn on the card. "panel" fields have no room on a fixed-size card. */
export type Slot = "title" | "duration" | "state" | "changes" | "repos" | "activity" | "size" | "panel";

/** Placed by key and kind, as `slot_of`: the daemon says which fields exist, the client lays them out. */
export function slotOf(def: FieldDef): Slot {
  switch (def.key) {
    case "repos":
      return "repos";
    case "title":
      return "title";
    case "activity":
      return "activity";
    case "has_changes":
      return "changes";
  }
  switch (def.kind) {
    case "duration":
      return "duration";
    case "badge":
      return "state";
    case "bytes":
      return "size";
    default:
      return "panel";
  }
}

/** Why it stopped, then why it is blocked, then paused; a badge this client does not know comes last. */
function stateRank(key: string) {
  return ({ stop_cause: 0, blocked_reason: 1, paused: 2 } as Record<string, number>)[key] ?? 3;
}

function hasValue(agent: Agent, def: FieldDef) {
  const v = fieldValue(agent, def.key);
  if (Array.isArray(v)) return v.length > 0;
  return v !== null && v !== undefined && v !== "" && v !== false;
}

/** The schema fields a card shows for `agent`: their condition holds and they have a value. */
export function shownFields(fields: FieldDef[], agent: Agent): FieldDef[] {
  return fields.filter((f) => evalCondition(f.show_when, agent) && hasValue(agent, f));
}

/** A badge field's items, as Lite's `badge_items`: a flag reads as its label, a list as one item each, and each takes
 * the schema's label and tone for its value. A blocked reason with no tone of its own reads as Attention. */
export function badgeItems(field: FieldDef, agent: Agent): { text: string; tone: Tone | null }[] {
  const v = fieldValue(agent, field.key);
  const raws = v === true ? [field.label] : (Array.isArray(v) ? v : [v]).map(String);
  return raws.map((raw) => ({
    text: field.value_labels?.[raw] ?? raw,
    tone: field.value_tones?.[raw] ?? (field.key === "blocked_reason" ? "attention" : null),
  }));
}

/** The card's one state badge: the shown badge field of lowest rank, and its first item. */
export function stateBadge(fields: FieldDef[], agent: Agent): { field: FieldDef; text: string; tone: Tone | null } | null {
  const field = shownFields(fields, agent)
    .filter((f) => slotOf(f) === "state")
    .sort((a, b) => stateRank(a.key) - stateRank(b.key))[0];
  const first = field && badgeItems(field, agent)[0];
  return field && first ? { field, ...first } : null;
}

/** The first repo and "+N" for the others; both name every repo on hover. */
export function repoSummary(names: string[]): { first: string; more: string | null; all: string } | null {
  const [first] = names;
  if (first === undefined) return null;
  return { first, more: names.length > 1 ? `+${names.length - 1}` : null, all: names.join("\n") };
}

/** The glyph a known action is drawn with; any other action is a text button, so nothing the schema offers is lost. */
export function iconOf(action: ActionDef): IconName | null {
  return action.id === "stop" || action.id === "pause" || action.id === "resume" ? action.id : null;
}

const wallSeconds = () => performance.now() / 1000;

/**
 * Lite's two-click confirm: a destructive action's first press arms it, the second runs it. `card.confirmFor`
 * seconds or a press anywhere else disarm it.
 */
export function useConfirm(now: () => number = wallSeconds) {
  const armed = useRef<{ id: string; since: number } | null>(null);
  const [, redraw] = useReducer((n: number) => n + 1, 0);

  const live = () => {
    if (armed.current && now() - armed.current.since > card.confirmFor) armed.current = null;
    return armed.current;
  };

  useEffect(() => {
    const current = armed.current;
    if (!current) return;
    const t = setTimeout(redraw, Math.max(0, (current.since + card.confirmFor - now()) * 1000) + 50);
    // A click arms after its own pointerdown, so listening now only sees later presses. The armed button carries
    // `data-confirming`, so pressing it again confirms rather than disarms.
    const outside = (e: PointerEvent) => {
      if (e.target instanceof Element && e.target.closest("[data-confirming]")) return;
      armed.current = null;
      redraw();
    };
    document.addEventListener("pointerdown", outside);
    return () => {
      clearTimeout(t);
      document.removeEventListener("pointerdown", outside);
    };
  });

  return {
    armed: (id: string) => live()?.id === id,
    /** True when the action should run now. */
    press: (id: string, destructive: boolean) => {
      if (!destructive || live()?.id === id) {
        armed.current = null;
        redraw();
        return true;
      }
      armed.current = { id, since: now() };
      redraw();
      return false;
    },
    disarm: () => {
      armed.current = null;
      redraw();
    },
  };
}
