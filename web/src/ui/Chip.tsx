import type { ReactNode } from "react";

/** Lite's `chip`: a toggle that, selected, takes the accent's tint and outline. `radio` makes it one choice of a
 * `radiogroup`, drawn the same. */
export function Chip(props: { selected: boolean; onClick: () => void; children: ReactNode; disabled?: boolean; title?: string; radio?: boolean }) {
  const { selected, onClick, children, disabled, title, radio } = props;
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
    >
      {children}
    </button>
  );
}
