import { useEffect, useRef, type ReactNode } from "react";

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
  useEffect(() => {
    if (!closable) return;
    const key = (e: KeyboardEvent) => e.key === "Escape" && close.current?.();
    document.addEventListener("keydown", key);
    return () => document.removeEventListener("keydown", key);
  }, [closable]);
  return (
    <div className="dialog-backdrop" onMouseDown={(e) => outside && e.target === e.currentTarget && close.current?.()}>
      <section className={className} role="dialog" aria-modal="true" aria-label={label} aria-labelledby={labelledBy}>
        {children}
      </section>
    </div>
  );
}
