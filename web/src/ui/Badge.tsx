import type { ReactNode } from "react";

interface Props {
  /** Tooltip, and the name of what the badge shows. */
  label: string;
  /** The schema field key, so the stylesheet can colour a specific badge. */
  field?: string;
  children: ReactNode;
}

export function Badge({ label, field, children }: Props) {
  return (
    <span className="badge" title={label} data-field={field}>
      {children}
    </span>
  );
}
