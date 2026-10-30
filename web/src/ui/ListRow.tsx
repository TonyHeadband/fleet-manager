import type { ReactNode } from "react";

interface Props {
  /** The item's name, which names the row for assistive tech. */
  name: string;
  selected: boolean;
  onSelect: () => void;
  /** Drawn dimmed, as egui's archived rows. */
  dimmed?: boolean;
  children: ReactNode;
}

/** egui's `list_row`: a selectable row with the selection fade, its content laid out by the caller. */
export function ListRow({ name, selected, onSelect, dimmed, children }: Props) {
  return (
    <button
      type="button"
      className="list-row"
      aria-label={name}
      aria-pressed={selected}
      data-dimmed={dimmed || undefined}
      onClick={onSelect}
    >
      {children}
    </button>
  );
}
