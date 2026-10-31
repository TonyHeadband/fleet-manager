import { useLayoutEffect, useState } from "react";
import { motion } from "../generated/layout";

/** Motion runs unless the system asks for less or Settings → Motion is off. */
export function motionOn(): boolean {
  if (document.documentElement.dataset.animations === "off") return false;
  return !window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
}

/** When, in ms from the entry's start, the entry's last animation ends: the footer's wipe or the last staggered card. */
export function entryEnd(cards: number): { last: "footer" | "card"; at: number } {
  const footer = motion.atFooter + motion.wipe;
  if (cards === 0) return { last: "footer", at: footer };
  const card = motion.atBoard + motion.atCards + (Math.min(cards, motion.staggerCap) - 1) * motion.stagger + motion.stepIn;
  return card > footer ? { last: "card", at: card } : { last: "footer", at: footer };
}

export interface Entry {
  /** The view is wiping in. */
  entering: boolean;
  /** The detail panel was open when the entry started, so it wipes in with the view. */
  panelEnters: boolean;
  /** A view change, not the first load: the entry waits for the drawer's marker to finish moving. */
  afterMarker: boolean;
  /** The entry's last animation ended. */
  end: () => void;
}

/**
 * A view's entry: once per new `key` (the route), as soon as `ready` holds, unless motion is off. It ends when its
 * last animation does, never by a timer, so a panel opened afterwards never inherits it.
 */
export function useEntry(key: string, ready: boolean, panelOpen: boolean): Entry {
  const [entry, setEntry] = useState<{ key: string; on: boolean; panel: boolean; after: boolean } | null>(null);
  const due = ready && entry?.key !== key;
  useLayoutEffect(() => {
    // Read once, as the entry starts: a panel opened during it does not join.
    if (due) setEntry((e) => ({ key, on: motionOn(), panel: panelOpen, after: e !== null }));
    // eslint-disable-next-line react-hooks/exhaustive-deps -- panelOpen counts only at the start
  }, [due, key]);
  const entering = !due && !!entry && entry.key === key && entry.on;
  return {
    entering,
    panelEnters: entering && !!entry?.panel,
    afterMarker: entering && !!entry?.after,
    end: () => setEntry((e) => e && { ...e, on: false }),
  };
}
