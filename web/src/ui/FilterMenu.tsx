import { useEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";

interface Props {
  /** The mono label before the summary, as Lite's combo shows it ("Repo"); none for a plain combo. */
  label?: string;
  /** What is picked, in words ("All repos"). */
  summary: string;
  /** The control's accessible name ("Repo filter"); none where a `<label>` names it by `id`. */
  name?: string;
  /** For a `<label htmlFor>`. */
  id?: string;
  /** A form's single choice: the button is a combobox and the menu a listbox of options. */
  listbox?: boolean;
  /** The menu's narrowest, from the generated layout tokens; `fill` matches the button, for a control as wide as its column. */
  width: "menu" | "release" | "fill";
  disabled?: boolean;
  /** The hover text, such as why it is disabled. */
  title?: string;
  /** The rows are a single choice of actions (`menuitem`s), not toggles. */
  actions?: boolean;
  /** Drawn before the label, such as the persona's avatar. */
  lead?: ReactNode;
  /** The label over the summary, on two lines, rather than beside it. */
  stacked?: boolean;
  /** The menu's rows; `close` closes it, for a single-choice menu that closes on a pick. */
  children: (close: () => void) => ReactNode;
}

/**
 * Lite's filter combo: the label and summary on a button, and a menu under it that closes on a click outside or
 * Escape. A pick inside leaves it open unless the rows close it, so several repos are picked in one go.
 */
export function FilterMenu({ label, summary, name, id, listbox, width, disabled, title, actions, lead, stacked, children }: Props) {
  // Where the menu opens: under the button, fixed to the window, since the header clips what overflows it.
  const [open, setOpen] = useState<{ top: number; left: number; width?: number } | null>(null);
  const root = useRef<HTMLDivElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const outside = (e: PointerEvent) => {
      const inside = e.target instanceof Node && (root.current?.contains(e.target) || menu.current?.contains(e.target));
      if (!inside) setOpen(null);
    };
    // Caught on the way down and stopped there: Escape closes the open menu, not the dialog it sits in.
    const key = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      setOpen(null);
    };
    document.addEventListener("pointerdown", outside);
    window.addEventListener("keydown", key, true);
    return () => {
      document.removeEventListener("pointerdown", outside);
      window.removeEventListener("keydown", key, true);
    };
  }, [open]);
  return (
    <div className="filter-menu" ref={root}>
      <button
        type="button"
        id={id}
        className="filter-button"
        role={listbox ? "combobox" : undefined}
        aria-label={name}
        aria-haspopup={listbox ? "listbox" : "true"}
        aria-expanded={open !== null}
        disabled={disabled}
        title={title}
        onClick={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          setOpen((o) => (o ? null : { top: r.bottom, left: r.left, width: width === "fill" ? r.width : undefined }));
        }}
      >
        {lead}
        {stacked ? (
          <span className="filter-lines">
            <span className="mono-label">{label}</span> <span className="filter-summary">{summary}</span>
          </span>
        ) : (
          <>
            {label && (
              <>
                <span className="mono-label">{label}</span>{" "}
              </>
            )}
            {summary}
          </>
        )}
      </button>
      {open &&
        // On the window, not in place: a sliding pane's transform would become the fixed menu's frame and its overflow
        // would clip it. In a dialog, on its backdrop: above the dialog, outside the card's clip, still in the modal.
        createPortal(
          <div ref={menu} className="menu" role={listbox ? "listbox" : actions ? "menu" : undefined} data-width={width} style={open}>
            {children(() => setOpen(null))}
          </div>,
          root.current?.closest(".dialog-backdrop") ?? document.body,
        )}
    </div>
  );
}
