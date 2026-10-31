import { useEffect, useLayoutEffect, useRef, type ReactNode } from "react";
import { motion } from "../generated/layout";
import { END_SLACK, motionOn } from "../shell/entry";

/**
 * A dialog closing, however it was closed (its own buttons, Escape, a press outside): a still copy of it stays over
 * the window and plays the opening backwards, then goes. The dialog itself is already gone, so nothing in it runs.
 */
function closeOut(backdrop: HTMLElement) {
  if (!motionOn()) return;
  const copy = backdrop.cloneNode(true) as HTMLElement;
  copy.dataset.closing = "";
  copy.setAttribute("inert", "");
  copy.setAttribute("aria-hidden", "true");
  copy.querySelectorAll("[id]").forEach((e) => e.removeAttribute("id"));
  document.body.append(copy);
  const done = () => {
    clearTimeout(t);
    copy.remove();
  };
  copy.addEventListener("animationend", (e) => e.animationName === "fui-dialog-close" && done());
  const t = setTimeout(done, motion.dialogClose + END_SLACK);
}

/**
 * egui's `Modal`: over the window, closed by Escape or a press outside. On the press, not the click: a text selection
 * dragged out of the card must not close it. `onClose` null: only the card's own buttons end it (a wizard's step, a
 * stop under way); `outside` false: Escape closes it, a press outside does not. `className` is the card's whole class.
 */
export function Modal({
  label,
  labelledBy,
  className = "dialog",
  onClose,
  outside = true,
  children,
}: {
  label?: string;
  labelledBy?: string;
  className?: string;
  onClose: (() => void) | null;
  outside?: boolean;
  children: ReactNode;
}) {
  // The latest: callers pass a fresh closure each render, which must not re-listen.
  const close = useRef(onClose);
  close.current = onClose;
  const closable = onClose !== null;
  const backdrop = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const el = backdrop.current;
    // A microtask on: StrictMode's development unmount keeps the backdrop in the page, a real close has removed it.
    return () => queueMicrotask(() => el && !el.isConnected && closeOut(el));
  }, []);
  useEffect(() => {
    if (!closable) return;
    const key = (e: KeyboardEvent) => e.key === "Escape" && close.current?.();
    document.addEventListener("keydown", key);
    return () => document.removeEventListener("keydown", key);
  }, [closable]);
  return (
    <div ref={backdrop} className="dialog-backdrop" onMouseDown={(e) => outside && e.target === e.currentTarget && close.current?.()}>
      <section className={className} role="dialog" aria-modal="true" aria-label={label} aria-labelledby={labelledBy}>
        {children}
      </section>
    </div>
  );
}
