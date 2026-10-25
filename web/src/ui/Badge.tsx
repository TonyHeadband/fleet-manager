import type { ReactNode } from "react";

interface Props {
  /** Tooltip, and the name of what the badge shows. */
  label: string;
  /** The schema field key, so the stylesheet can colour a specific badge. */
  field?: string;
  /** The schema's tone for this value, if it gives one. */
  tone?: string;
  children: ReactNode;
}

export function Badge({ label, field, tone, children }: Props) {
  return (
    <span className="badge" title={label} data-field={field} data-tone={tone}>
      {children}
    </span>
  );
}
