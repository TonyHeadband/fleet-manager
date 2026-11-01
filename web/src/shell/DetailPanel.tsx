import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { layout, motion } from "../generated/layout";
import { END_SLACK, motionOn } from "./entry";
import { layoutOf, type Panel } from "./panel";
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
  /** Whether it covers the board now: full, and done widening over it. */
  onCover?: (covering: boolean) => void;
}

export type PanelPhase = "opening" | "open" | "closing";

/**
 * The detail panel: at the board's right, a phone's width, the quiet column a rail beside it (panel.ts). It covers
 * the board when the person asks, when the open tab wants the room, or when the window is too narrow to keep the
 * board, widening over it and narrowing back; Esc leaves a fullscreen that was not forced.
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
  onCover,
}: Props) {
  const mode = layoutOf(panel, boardWidth);
  const path = panelPath(agent);
  const blocked = terminalBlocked(agent, daemonHost, onTerminal !== null);
  const forced = mode.kind === "full" && mode.forced;
  const { chosenFull, setChosenFull, wide, setWide } = panel;
  const back = () => {
    setChosenFull(false);
    setWide(false);
  };

  useEffect(() => {
    if (!(chosenFull || wide) || forced) return;
    // As in Lite, Esc is left alone while the person types or a dialog is up: it belongs to them.
    const key = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || typing(e.target) || document.querySelector('[role="dialog"][aria-modal="true"]')) return;
      back();
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  });

  // Going full, it is drawn full at once and widens over the board, which shows until it is covered; going back,
  // it stays drawn full while it narrows, then sits beside the board again. Without motion, both happen at once.
  const [drawn, setDrawn] = useState(mode.kind);
  const [resize, setResize] = useState<"widen" | "narrow" | null>(null);
  const [lastKind, setLastKind] = useState(mode.kind);
  if (lastKind !== mode.kind) {
    setLastKind(mode.kind);
    const animate = motionOn() && phase === "open";
    if (mode.kind === "full") {
      setDrawn("full");
      setResize(animate ? "widen" : null);
    } else if (animate) {
      setResize("narrow");
    } else {
      setDrawn("side");
      setResize(null);
    }
  }
  const settle = () => {
    if (resize === "narrow") setDrawn("side");
    setResize(null);
  };
  // A lost end (the panel hidden mid-motion) still settles it.
  useEffect(() => {
    if (!resize) return;
    const t = setTimeout(settle, motion.push + END_SLACK);
    return () => clearTimeout(t);
  });
  const covering = drawn === "full" && resize === null;
  useEffect(() => onCover?.(covering), [covering, onCover]);
  useEffect(() => () => onCover?.(false), [onCover]);

  const width = layout.panel;
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

  return (
    <aside
      ref={aside}
      className="detail outlined"
      aria-label="Detail"
      data-layout={drawn}
      data-resize={resize ?? undefined}
      data-enters={enters || undefined}
      data-phase={phase}
      onAnimationEnd={(e) => {
        if (e.target !== e.currentTarget) return;
        if (e.animationName === "fui-widen" || e.animationName === "fui-narrow") settle();
        else onPhaseEnd?.(e.animationName);
      }}
      // --panel-final: the contents keep the panel's final width while it moves, so nothing reflows inside; widening,
      // that is the whole section. A widening starts and a narrowing ends at --side-width (shell.css).
      style={
        (drawn === "side"
          ? { width: `${width}px`, "--panel-final": `${width}px`, ...reverse }
          : boardWidth === null
            ? {}
            : { "--panel-final": `${boardWidth}px` }) as CSSProperties
      }
    >
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
        {drawn === "side" ? (
          <SmallIconButton icon="expand" label="Fullscreen" large onClick={() => setChosenFull(true)} />
        ) : (
          <SmallIconButton
            icon="collapse"
            label="Back to the board"
            title={forced ? "the window is too narrow for the board" : undefined}
            disabled={forced}
            large
            onClick={back}
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
