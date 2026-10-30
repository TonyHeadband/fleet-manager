import { useEffect, useRef, useState, type ReactNode } from "react";

interface Props {
  /** The mono label before the summary, as Lite's combo shows it ("Repo"). */
  label: string;
  /** What is picked, in words ("All repos"). */
  summary: string;
  /** The control's accessible name ("Repo filter"). */
  name: string;
  /** The menu's narrowest, from the generated layout tokens. */
  width: "menu" | "release";
  /** The menu's rows; `close` closes it, for a single-choice menu that closes on a pick. */
  children: (close: () => void) => ReactNode;
}

/**
 * Lite's filter combo: the label and summary on a button, and a menu under it that closes on a click outside or
 * Escape. A pick inside leaves it open unless the rows close it, so several repos are picked in one go.
 */
export function FilterMenu({ label, summary, name, width, children }: Props) {
  // Where the menu opens: under the button, fixed to the window, since the header clips what overflows it.
  const [open, setOpen] = useState<{ top: number; left: number } | null>(null);
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const outside = (e: PointerEvent) => {
      if (!(e.target instanceof Node && root.current?.contains(e.target))) setOpen(null);
    };
    const key = (e: KeyboardEvent) => e.key === "Escape" && setOpen(null);
    document.addEventListener("pointerdown", outside);
    document.addEventListener("keydown", key);
    return () => {
      document.removeEventListener("pointerdown", outside);
      document.removeEventListener("keydown", key);
    };
  }, [open]);
  return (
    <div className="filter-menu" ref={root}>
      <button type="button" className="filter-button" aria-label={name} aria-haspopup="true" aria-expanded={open !== null}
        onClick={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          setOpen((o) => (o ? null : { top: r.bottom, left: r.left }));
        }}
      >
        <span className="mono-label">{label}</span> {summary}
      </button>
      {open && (
        <div className="menu" data-width={width} style={open}>
          {children(() => setOpen(null))}
        </div>
      )}
    </div>
  );
}
