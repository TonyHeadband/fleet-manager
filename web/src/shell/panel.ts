import { useEffect, useState } from "react";
import { layout } from "../generated/layout";

// The detail panel's rules. Beside the board it takes the quiet column's place: that column becomes a rail and the
// panel is the rest of its share, so the other columns keep their width. It covers the board when the person asks,
// when the open tab wants the room (the Diff), or when that share is too narrow.

export type PanelLayout = { kind: "side" } | { kind: "full"; forced: boolean };

/** The side panel's width in a body this wide shared by `columns` columns: one column's share (inside the board's 1px outline), less the rail and the canvas gap. */
export function sideWidth(bodyWidth: number, columns: number) {
  return (bodyWidth - 2) / columns - layout.rail - layout.canvasGap;
}

export function panelLayout(bodyWidth: number, columns: number, full: boolean): PanelLayout {
  if (sideWidth(bodyWidth, columns) < layout.panelSideMin) return { kind: "full", forced: true };
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
export function layoutOf(panel: Panel, bodyWidth: number | null, columns: number): PanelLayout {
  const full = panel.chosenFull || panel.wide;
  return bodyWidth === null ? (full ? { kind: "full", forced: false } : { kind: "side" }) : panelLayout(bodyWidth, columns, full);
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
