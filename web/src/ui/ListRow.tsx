import type { ReactNode } from "react";

interface Props {
  /** The item's name, which names the row for assistive tech; without it the row's content names it. */
  name?: string;
  selected: boolean;
  onSelect: () => void;
  /** How the selection is told: a toggle with the selection fade, or the open item (`aria-current`) in a 1px accent
   * outline, for a list beside what it opens. */
  mark?: "pressed" | "current";
  /** Drawn dimmed, as egui's archived rows. */
  dimmed?: boolean;
  children: ReactNode;
}

/** egui's `list_row`: a selectable row with the selection fade, its content laid out by the caller. */
export function ListRow({ name, selected, onSelect, mark = "pressed", dimmed, children }: Props) {
  return (
    <button
      type="button"
      className="list-row"
      aria-label={name}
      aria-pressed={mark === "pressed" ? selected : undefined}
      aria-current={mark === "current" && selected ? "true" : undefined}
      data-dimmed={dimmed || undefined}
      onClick={onSelect}
    >
      {children}
    </button>
  );
}
