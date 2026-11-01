import type { ReactNode } from "react";

/** Lite's `chip`: a toggle that, selected, takes the accent's tint and outline. `radio` makes it one choice of a
 * `radiogroup`, drawn the same; its group sets `tabIndex` for the roving focus and moves it with the arrow keys. */
export function Chip(props: { selected: boolean; onClick: () => void; children: ReactNode; disabled?: boolean; title?: string; radio?: boolean; tabIndex?: number }) {
  const { selected, onClick, children, disabled, title, radio, tabIndex } = props;
  return (
    <button
      type="button"
      className="chip"
      role={radio ? "radio" : undefined}
      aria-checked={radio ? selected : undefined}
      aria-pressed={radio ? undefined : selected}
      onClick={onClick}
      disabled={disabled}
      title={title}
      tabIndex={tabIndex}
    >
      {children}
    </button>
  );
}
