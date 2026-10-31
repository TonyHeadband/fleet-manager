import { useLayoutEffect, useRef, type AnimationEvent, type ReactNode, type Ref } from "react";
import { motion } from "../generated/layout";
import { entryEnd, type Entry } from "./entry";

/** What steps in after the sections' wipe, in board order. */
const ITEMS = ".card, .fleet-row";

interface Props {
  drawer: ReactNode;
  /** The view's header row. */
  header: ReactNode;
  /** The header's telemetry, at its right end; the board alone has one. */
  readouts?: ReactNode;
  footer: ReactNode;
  /** The detail panel, beside the view's body. */
  panel?: ReactNode;
  /** A dialog over the whole window. */
  dialog?: ReactNode;
  /** The detail panel covers the view: fullscreen. */
  covered?: boolean;
  /** Measures the width the view and the panel share. */
  bodyRef?: Ref<HTMLDivElement>;
  /** The view's entry, keyed by its route: a new key remounts the sections, so an old view's animations end unheard. */
  entry?: Entry & { key: string };
  children: ReactNode;
}

/**
 * The FUI canvas: the drawer claims the left edge for the window's full height; beside it, on the page colour, the
 * header, the view's body (with the detail panel beside it) and the footer, each an outlined section. Every size is a
 * generated token.
 */
export function Shell({ drawer, header, readouts, footer, panel, dialog, covered, bodyRef, entry, children }: Props) {
  const content = useRef<HTMLDivElement>(null);
  const entering = !!entry?.entering;
  // Each card's place in the stagger, in board order (column by column, top to bottom); the cards after the cap step
  // in with the last one, so a large board ends when a small one does.
  useLayoutEffect(() => {
    if (!entering) return;
    content.current
      ?.querySelectorAll<HTMLElement>(ITEMS)
      .forEach((el, i) => el.style.setProperty("--i", String(Math.min(i, motion.staggerCap - 1))));
  });
  const ended = (e: AnimationEvent) => {
    if (!entering || !content.current) return;
    const items = content.current.querySelectorAll(ITEMS);
    const target = e.target as Element;
    const last =
      entryEnd(items.length).last === "footer"
        ? e.animationName === "fui-wipe" && target.classList.contains("footer")
        : e.animationName === "fui-step-in" && target === items[Math.min(items.length, motion.staggerCap) - 1];
    if (last) entry?.end();
  };
  return (
    <div className="shell">
      {drawer}
      <div
        className="content"
        key={entry?.key}
        ref={content}
        data-entering={entering || undefined}
        data-after-marker={entry?.afterMarker || undefined}
        onAnimationEnd={ended}
      >
        <header className="view-header outlined">
          {header}
          {readouts}
        </header>
        <div className="body" ref={bodyRef}>
          <main className="view-body outlined" hidden={covered}>
            {children}
          </main>
          {panel}
        </div>
        {footer}
      </div>
      {dialog}
    </div>
  );
}
