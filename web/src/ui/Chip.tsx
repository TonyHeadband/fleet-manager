import type { ReactNode } from "react";

/** Lite's `chip`: a toggle that, selected, takes the accent's tint and outline. */
export function Chip({ selected, onClick, children }: { selected: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <button type="button" className="chip" aria-pressed={selected} onClick={onClick}>
      {children}
    </button>
  );
}
