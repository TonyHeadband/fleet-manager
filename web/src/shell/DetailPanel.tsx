import { useEffect, useRef, type ReactNode } from "react";
import { dragWidth, layoutOf, shownWidth, type Panel } from "./panel";

interface Props {
  title: string;
  panel: Panel;
  /** The width the board and the panel share; `null` until measured. */
  boardWidth: number | null;
  onClose: () => void;
  children: ReactNode;
}

/**
 * Lite's detail panel: at the board's right, resized by dragging its 1px left line, between its minimum and a cap
 * that keeps a quarter of the board. It covers the board when the person asks, or when the window is too narrow to
 * keep the board, and Esc leaves a fullscreen the person chose.
 */
export function DetailPanel({ title, panel, boardWidth, onClose, children }: Props) {
  const mode = layoutOf(panel, boardWidth);
  const drag = useRef<{ x: number; width: number } | null>(null);
  const forced = mode.kind === "full" && mode.forced;
  const { chosenFull, setChosenFull } = panel;

  useEffect(() => {
    if (!chosenFull || forced) return;
    const key = (e: KeyboardEvent) => e.key === "Escape" && setChosenFull(false);
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [chosenFull, forced, setChosenFull]);

  const width = boardWidth === null ? panel.width : shownWidth(panel.width, boardWidth);
  const move = (x: number) => {
    if (!drag.current || boardWidth === null) return;
    // The handle is on the panel's left: dragging left widens it.
    panel.resize(dragWidth(drag.current.width, drag.current.x - x, boardWidth), boardWidth);
  };

  return (
    <aside className="detail" aria-label="Detail" data-layout={mode.kind} style={mode.kind === "side" ? { width: `${width}px` } : undefined}>
      {mode.kind === "side" && (
        <div
          className="detail-handle"
          role="separator"
          aria-orientation="vertical"
          aria-label="Resize the detail panel"
          onPointerDown={(e) => {
            drag.current = { x: e.clientX, width };
            e.currentTarget.setPointerCapture?.(e.pointerId);
          }}
          onPointerMove={(e) => move(e.clientX)}
          onPointerUp={(e) => {
            move(e.clientX);
            drag.current = null;
          }}
        />
      )}
      <div className="detail-top">
        <h2>{title}</h2>
        {mode.kind === "side" ? (
          <button type="button" className="ghost icon" aria-label="Fullscreen" title="Fullscreen" onClick={() => setChosenFull(true)}>
            ⤢
          </button>
        ) : (
          <button
            type="button"
            className="ghost icon"
            aria-label="Back to the board"
            title={forced ? "the window is too narrow for the board" : "Back to the board"}
            disabled={forced}
            onClick={() => setChosenFull(false)}
          >
            ⤡
          </button>
        )}
        <button type="button" className="ghost icon" aria-label="Close" title="Close" onClick={onClose}>
          ✕
        </button>
      </div>
      <div className="detail-body">{children}</div>
    </aside>
  );
}
