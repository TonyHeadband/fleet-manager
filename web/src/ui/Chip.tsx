import type { ReactNode } from "react";

/** Lite's `chip`: a toggle that, selected, takes the accent's tint and outline. */
export function Chip(props: { selected: boolean; onClick: () => void; children: ReactNode; disabled?: boolean; title?: string }) {
  const { selected, onClick, children, disabled, title } = props;
  return (
    <button type="button" className="chip" aria-pressed={selected} onClick={onClick} disabled={disabled} title={title}>
      {children}
    </button>
  );
}
