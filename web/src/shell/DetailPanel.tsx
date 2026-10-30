import { useEffect, useRef, type ReactNode } from "react";
import { dragWidth, layoutOf, shownWidth, type Panel } from "./panel";
import type { Agent } from "../api/generated/Agent";
import { title } from "../api/schema";
import { panelPath, terminalBlocked } from "../panel/header";
import { SmallIconButton } from "../ui/SmallIconButton";

const typing = (t: EventTarget | null) => t instanceof HTMLElement && (t.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(t.tagName));

interface Props {
  /** The agent the panel shows; its title, goal and path head the panel. */
  agent: Agent;
  panel: Panel;
  /** The width the board and the panel share; `null` until measured. */
  boardWidth: number | null;
  onClose: () => void;
  children: ReactNode;
  /** Between the title and the tabs: Lite's details and repos bands. */
  bands?: ReactNode;
  /** Pinned under the scrolling body, as Lite's bottom panel: the reply box. */
  reply?: ReactNode;
}

/**
 * Lite's detail panel: at the board's right, resized by dragging its 1px left line, between its minimum and a cap
 * that keeps a quarter of the board. It covers the board when the person asks, or when the window is too narrow to
 * keep the board, and Esc leaves a fullscreen the person chose.
 */
export function DetailPanel({ agent, panel, boardWidth, onClose, children, bands, reply }: Props) {
  const mode = layoutOf(panel, boardWidth);
  const path = panelPath(agent);
  const drag = useRef<{ x: number; width: number } | null>(null);
  const forced = mode.kind === "full" && mode.forced;
  const { chosenFull, setChosenFull } = panel;

  useEffect(() => {
    if (!chosenFull || forced) return;
    // As in Lite, Esc is left alone while the person types or a dialog is up: it belongs to them.
    const key = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || typing(e.target) || document.querySelector('[role="dialog"][aria-modal="true"]')) return;
      setChosenFull(false);
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [chosenFull, forced, setChosenFull]);

  const width = boardWidth === null ? panel.width : shownWidth(panel.width, boardWidth);
  const move = (x: number, buttons: number) => {
    // A drag whose release was lost (a window switch mid-drag) ends here rather than following the pointer.
    if (buttons === 0) drag.current = null;
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
            // No text selection across the board while dragging.
            e.preventDefault();
            drag.current = { x: e.clientX, width };
            e.currentTarget.setPointerCapture?.(e.pointerId);
          }}
          onPointerMove={(e) => move(e.clientX, e.buttons)}
          onPointerUp={(e) => {
            move(e.clientX, 1);
            drag.current = null;
          }}
          onPointerCancel={() => (drag.current = null)}
          onLostPointerCapture={() => (drag.current = null)}
        />
      )}
      <div className="detail-top">
        <span className="detail-path" title={path}>
          {path}
        </span>
        <SmallIconButton icon="terminal" label="Open in terminal" title={terminalBlocked(agent, window.location.hostname)} disabled large onClick={() => {}} />
        {mode.kind === "side" ? (
          <SmallIconButton icon="expand" label="Fullscreen" large onClick={() => setChosenFull(true)} />
        ) : (
          <SmallIconButton
            icon="collapse"
            label="Back to the board"
            title={forced ? "the window is too narrow for the board" : undefined}
            disabled={forced}
            large
            onClick={() => setChosenFull(false)}
          />
        )}
        <SmallIconButton icon="close" label="Close" large onClick={onClose} />
      </div>
      <div className="detail-title">
        <h2>{title(agent)}</h2>
        {agent.goal !== title(agent) && <p className="detail-goal">{agent.goal}</p>}
      </div>
      {bands}
      <div className="detail-body">{children}</div>
      {reply}
    </aside>
  );
}
