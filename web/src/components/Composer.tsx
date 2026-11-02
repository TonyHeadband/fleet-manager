import { useEffect, useRef, useState, type ReactNode } from "react";
import { PlusMinus } from "../ui/PlusMinus";

interface Props {
  /** The header's name for what is written: "Message", "Post". */
  label: string;
  /** Where little is written (a room), it starts folded to its header. */
  initiallyOpen: boolean;
  /** Unsent text is waiting: a folded header says so, so a draft is never out of sight unnoticed. */
  draft: boolean;
  children: ReactNode;
}

/** The writing area under a chat, folding to one header line, as a chat's call sections do. */
export function Composer({ label, initiallyOpen, draft, children }: Props) {
  const [open, setOpen] = useState(initiallyOpen);
  const body = useRef<HTMLDivElement>(null);
  const opened = useRef(false);
  useEffect(() => {
    // Opening it is asking to write: the cursor goes to the box. Never on the first draw, which would steal it.
    if (open && opened.current) body.current?.querySelector("textarea")?.focus();
    opened.current = true;
  }, [open]);
  return (
    <div className="composer">
      <button type="button" className="composer-head" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        <PlusMinus open={open} />
        <span>{label}</span>
        {!open && draft && <span className="composer-draft">· draft</span>}
      </button>
      {open && (
        <div ref={body} className="composer-body">
          {children}
        </div>
      )}
    </div>
  );
}
