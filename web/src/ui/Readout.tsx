import type { ReactNode, Ref } from "react";
import type { Tone } from "../api/generated/Tone";

/** A count as a readout shows it: at least two digits. */
export const pad2 = (n: number) => String(n).padStart(2, "0");

interface Props {
  /** The key, shown in mono caps. */
  k: string;
  /** A status bar between the key and the value, in this tone. */
  tone?: Tone;
  /** The whole cell is a link. */
  href?: string;
  title?: string;
  /** Names the value for assistive tech where it is padded or abbreviated. */
  label?: string;
  /** Gives way on a rail too narrow for it. */
  hidden?: boolean;
  children?: ReactNode;
}

/** One `KEY ▌value` cell of a readout rail. */
export function Readout({ k, tone, href, title, label, hidden, children }: Props) {
  const body = (
    <>
      <span className="readout-key mono-label">{k}</span>
      {tone && <span className="dot" data-tone={tone} aria-hidden="true" />}
      <span className="readout-value" aria-label={label}>
        {children}
      </span>
    </>
  );
  return href ? (
    <a className="readout" href={href} title={title} hidden={hidden}>
      {body}
    </a>
  ) : (
    <span className="readout" title={title} hidden={hidden}>
      {body}
    </span>
  );
}

/** A row of readouts, divided by hairlines: the header's and the footer's telemetry. */
export function ReadoutRail({ label, ref, children }: { label: string; ref?: Ref<HTMLDivElement>; children: ReactNode }) {
  return (
    <div ref={ref} className="readout-rail" role="group" aria-label={label}>
      {children}
    </div>
  );
}
