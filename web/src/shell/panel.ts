import { useEffect, useState } from "react";
import { layout } from "../generated/layout";

// The detail panel's rules, from Lite (preview.rs `panel_layout`). Beside the board it is a phone's width, and the
// board's quiet column becomes a rail; it covers the board when the person asks, when the open tab wants the room (the
// Diff), or when less than a quarter of the board would stay.

export type PanelLayout = { kind: "side" } | { kind: "full"; forced: boolean };

export function panelLayout(bodyWidth: number, full: boolean): PanelLayout {
  if (bodyWidth - layout.panel < bodyWidth * layout.boardKeep) return { kind: "full", forced: true };
  return full ? { kind: "full", forced: false } : { kind: "side" };
}

export interface Panel {
  chosenFull: boolean;
  setChosenFull: (full: boolean) => void;
  /** The open tab wants the whole section (the Diff); Back to the board or Esc sets that aside until the tab changes. */
  wide: boolean;
  setWide: (wide: boolean) => void;
}

export function usePanel(): Panel {
  const [chosenFull, setChosenFull] = useState(false);
  const [wide, setWide] = useState(false);
  return { chosenFull, setChosenFull, wide, setWide };
}

/** The panel's layout for a body this wide; `null` while the width is not measured yet, which keeps the panel at the side. */
export function layoutOf(panel: Panel, bodyWidth: number | null): PanelLayout {
  const full = panel.chosenFull || panel.wide;
  return bodyWidth === null ? (full ? { kind: "full", forced: false } : { kind: "side" }) : panelLayout(bodyWidth, full);
}

/** An element's width, kept current; `null` until measured, or where there is no ResizeObserver (tests). */
export function useWidth<T extends HTMLElement>(): [(el: T | null) => void, number | null] {
  const [el, setEl] = useState<T | null>(null);
  const [width, setWidth] = useState<number | null>(null);
  useEffect(() => {
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(([entry]) => entry && setWidth(entry.contentRect.width));
    ro.observe(el);
    return () => ro.disconnect();
  }, [el]);
  return [setEl, width];
}
