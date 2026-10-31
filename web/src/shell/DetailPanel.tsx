import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { motion } from "../generated/layout";
import { dragWidth, layoutOf, shownWidth, type Panel } from "./panel";
import type { Agent } from "../api/generated/Agent";
import { title } from "../api/schema";
import { panelPath, terminalBlocked } from "../panel/header";
import { SmallIconButton } from "../ui/SmallIconButton";

const typing = (t: EventTarget | null) =>
  t instanceof HTMLElement && (t.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(t.tagName));

interface Props {
  /** The agent the panel shows; its title, goal and path head the panel. */
  agent: Agent;
  panel: Panel;
  /** The width the board and the panel share; `null` until measured. */
  boardWidth: number | null;
  /** The daemon's host, for Open in terminal's reasons. */
  daemonHost: string;
  /** Opens the agent's terminal; null where none can be opened (a browser, a daemon elsewhere). */
  onTerminal: (() => void) | null;
  onClose: () => void;
  children: ReactNode;
  /** Between the title and the tabs: Lite's details and repos bands. */
  bands?: ReactNode;
  /** Pinned under the scrolling body, as Lite's bottom panel: the reply box. */
  reply?: ReactNode;
  /** It was open as the view's entry started, so it wipes in with the view. */
  enters?: boolean;
  /** Pushing in, standing, or pulling out; the push and pull animate the side layout only. */
  phase?: PanelPhase;
  /** One of the panel's own animations ended, by name. */
  onPhaseEnd?: (animation: string) => void;
}

export type PanelPhase = "opening" | "open" | "closing";

/**
 * Lite's detail panel: at the board's right, resized by dragging its 1px left line, between its minimum and a cap
 * that keeps a quarter of the board. It covers the board when the person asks, or when the window is too narrow to
 * keep the board, and Esc leaves a fullscreen the person chose.
 */
export function DetailPanel({
  agent,
  panel,
  boardWidth,
  daemonHost,
  onTerminal,
  onClose,
  children,
  bands,
  reply,
  enters,
  phase = "open",
  onPhaseEnd,
}: Props) {
  const mode = layoutOf(panel, boardWidth);
  const path = panelPath(agent);
  const blocked = terminalBlocked(agent, daemonHost, onTerminal !== null);
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
  // Reversed mid-motion (pressed again while pulling out, closed while pushing in), the panel moves on from the width
  // it had reached, for the share of the time left, rather than jumping to the keyframes' far end first.
  const aside = useRef<HTMLElement>(null);
  const [lastPhase, setLastPhase] = useState(phase);
  const [reverse, setReverse] = useState<CSSProperties>({});
  if (lastPhase !== phase) {
    setLastPhase(phase);
    const at = aside.current?.getBoundingClientRect().width;
    const reversed = (lastPhase === "opening" && phase === "closing") || (lastPhase === "closing" && phase === "opening");
    const share = at === undefined ? 0 : Math.min(at / width, 1);
    setReverse(
      !reversed || at === undefined
        ? {}
        : phase === "closing"
          ? ({ "--pull-from": `${at}px`, "--pull-time": `${Math.round(motion.pull * share)}ms` } as CSSProperties)
          : ({ "--push-from": `${at}px`, "--push-time": `${Math.round(motion.push * (1 - share))}ms` } as CSSProperties),
    );
  }
  const move = (x: number, buttons: number) => {
    // A drag whose release was lost (a window switch mid-drag) ends here rather than following the pointer.
    if (buttons === 0) drag.current = null;
    if (!drag.current || boardWidth === null) return;
    // The handle is on the panel's left: dragging left widens it.
    panel.resize(dragWidth(drag.current.width, drag.current.x - x, boardWidth), boardWidth);
  };

  return (
    <aside
      ref={aside}
      className="detail outlined"
      aria-label="Detail"
      data-layout={mode.kind}
      data-enters={enters || undefined}
      data-phase={phase}
      onAnimationEnd={(e) => {
        if (e.target === e.currentTarget) onPhaseEnd?.(e.animationName);
      }}
      // --panel-final: the contents keep the panel's width while it pushes and pulls, so nothing reflows inside.
      style={mode.kind === "side" ? ({ width: `${width}px`, "--panel-final": `${width}px`, ...reverse } as CSSProperties) : undefined}
    >
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
        <SmallIconButton
          icon="terminal"
          label="Open in terminal"
          title={blocked ?? undefined}
          disabled={blocked !== null}
          large
          onClick={() => onTerminal?.()}
        />
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
