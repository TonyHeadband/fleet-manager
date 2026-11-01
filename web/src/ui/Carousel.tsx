import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { motion } from "../generated/layout";
import { END_SLACK, motionOn } from "../shell/entry";

interface Props<Id extends string> {
  /** Every pane's id, in the tabs' order: the track's order. */
  ids: readonly Id[];
  current: Id;
  /**
   * What stays off to the side: every pane visited, mounted (the detail panel's tabs, so the Diff loads once and the
   * chat keeps following), or, as it slides away, only a still copy of the pane just left (a view's tabs, whose
   * pages stop their work the moment they are left).
   */
  keep: "visited" | "sliding";
  render: (id: Id) => ReactNode;
}

/** A still copy of a pane's DOM, its scroll positions with it. */
function still(slot: HTMLElement): { node: HTMLElement; scrolls: number[] } {
  const node = document.createElement("div");
  node.className = "pane-still";
  node.append(...[...slot.childNodes].map((n) => n.cloneNode(true)));
  return { node, scrolls: [...slot.querySelectorAll("*")].map((e) => e.scrollTop) };
}

/**
 * Panes side by side on a track, each the carousel's full width and scrolling on its own; the track slides to the
 * current pane (motion.css), crossing those between, and the carousel clips the neighbours. Panes away are inert.
 */
export function Carousel<Id extends string>({ ids, current, keep, render }: Props<Id>) {
  const [visited, setVisited] = useState<ReadonlySet<Id>>(() => new Set([current]));
  const [last, setLast] = useState(current);
  const index = ids.indexOf(current);
  // A tab coming or going (the orchestrator's Breakdown) moves the current pane's place without a press: the track
  // jumps there for one frame with no transition, rather than sliding on its own.
  const [place, setPlace] = useState(index);
  const [jump, setJump] = useState(false);
  if (place !== index) {
    setPlace(index);
    setJump(last === current);
  }
  useEffect(() => {
    if (!jump) return;
    const f = requestAnimationFrame(() => setJump(false));
    return () => cancelAnimationFrame(f);
  }, [jump]);
  const slots = useRef(new Map<Id, HTMLDivElement>());
  // The pane being left, copied before React replaces it, and where the copy stands while the track slides.
  const leaving = useRef<{ id: Id; node: HTMLElement; scrolls: number[] } | null>(null);
  const shown = useRef<HTMLElement | null>(null);
  if (last !== current) {
    setLast(current);
    if (keep === "visited") setVisited(new Set([...visited, current]));
    const slot = slots.current.get(last);
    if (keep === "sliding" && motionOn() && slot) leaving.current = { id: last, ...still(slot) };
  }
  const settle = () => {
    shown.current?.remove();
    shown.current = null;
  };
  useLayoutEffect(() => {
    settle();
    const l = leaving.current;
    leaving.current = null;
    const slot = l && slots.current.get(l.id);
    if (!l || !slot) return;
    slot.append(l.node);
    l.node.querySelectorAll("*").forEach((e, i) => (e.scrollTop = l.scrolls[i] ?? 0));
    shown.current = l.node;
    // The slide's end removes the copy; a slide whose end never comes (cut short, or no transition) removes it too.
    const t = setTimeout(settle, motion.slide + END_SLACK);
    return () => clearTimeout(t);
  }, [current]);
  const kept = (id: Id) => id === current || (keep === "visited" && visited.has(id));
  return (
    <div className="carousel">
      <div
        className="pane-track"
        style={{ "--index": index } as CSSProperties}
        data-jump={jump || undefined}
        onTransitionEnd={(e) => {
          if (e.target === e.currentTarget) settle();
        }}
      >
        {ids.map((id) => {
          const away = id !== current;
          return (
            <div
              key={id}
              ref={(el) => {
                if (el) slots.current.set(id, el);
                else slots.current.delete(id);
              }}
              className="pane-slot"
              inert={away}
              aria-hidden={away || undefined}
            >
              {kept(id) && render(id)}
            </div>
          );
        })}
      </div>
    </div>
  );
}
