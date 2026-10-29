import { useCallback, useEffect, useState } from "react";
import { layout } from "../generated/layout";

// The detail panel's rules, ported from Lite (crates/fleet-gui/src/components/preview.rs `panel_layout` and the
// preview panel block in app.rs), with the sizes both read from theme.rs.

export type PanelLayout = { kind: "side" } | { kind: "full"; forced: boolean };

export function panelLayout(boardWidth: number, panelWidth: number, chosenFull: boolean): PanelLayout {
  if (boardWidth - panelWidth < boardWidth * layout.boardKeep) return { kind: "full", forced: true };
  return chosenFull ? { kind: "full", forced: false } : { kind: "side" };
}

/** The widest a drag can make the panel: short of the width that would force fullscreen, so a window that cannot grow never traps the person there. The 1px margin keeps rounding from flipping it. */
export function panelCap(boardWidth: number) {
  return boardWidth * layout.panelCap - 1;
}

const clamp = (width: number, boardWidth: number) => Math.min(Math.max(width, layout.panelMin), panelCap(boardWidth));

/** The width after a drag of `dx` (positive widens) from `start`. */
export function dragWidth(start: number, dx: number, boardWidth: number) {
  return clamp(start + dx, boardWidth);
}

/** The width the panel shows at: the chosen one, held under the cap by a shrinking window. */
export function shownWidth(width: number, boardWidth: number) {
  return clamp(width, boardWidth);
}

/** Only a width under the cap is the person's choice; at the cap it may be the one a shrinking window clamped it to, and only the chosen one lets a shrink force fullscreen. */
export function recordWidth(shown: number, cap: number, previous: number) {
  return shown < cap - 0.5 ? shown : previous;
}

export interface Panel {
  /** The width the person dragged to; kept for the session, as in Lite. */
  width: number;
  /** The width the fullscreen rule reads: `width`, unless that was taken at the cap. */
  recorded: number;
  chosenFull: boolean;
  setChosenFull: (full: boolean) => void;
  resize: (width: number, boardWidth: number) => void;
}

export function usePanel(): Panel {
  const [width, setWidth] = useState<number>(layout.panel);
  const [recorded, setRecorded] = useState<number>(layout.panel);
  const [chosenFull, setChosenFull] = useState(false);
  const resize = useCallback((w: number, boardWidth: number) => {
    setWidth(w);
    setRecorded((prev) => recordWidth(w, panelCap(boardWidth), prev));
  }, []);
  return { width, recorded, chosenFull, setChosenFull, resize };
}

/** The panel's layout for a board this wide; `null` while the width is not measured yet, which keeps the panel at the side. */
export function layoutOf(panel: Panel, boardWidth: number | null): PanelLayout {
  return boardWidth === null ? (panel.chosenFull ? { kind: "full", forced: false } : { kind: "side" }) : panelLayout(boardWidth, panel.recorded, panel.chosenFull);
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
