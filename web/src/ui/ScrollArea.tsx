import { useState, type ReactNode } from "react";
import { useScrollEdges } from "./scrollEdges";

interface Props {
  as?: "div" | "section" | "main";
  className: string;
  /** Takes the room its parent row or column leaves, as the scroller it wraps did. */
  grow?: boolean;
  label?: string;
  hidden?: boolean;
  children: ReactNode;
}

/**
 * A scroller in a frame: while it goes on below, the frame draws a tick rail along its bottom edge with a readout of
 * how far down it is.
 */
export function ScrollArea({ as: Tag = "div", className, grow, label, hidden, children }: Props) {
  // Only the readout re-renders the frame: a scroll that moves it by less than a percent changes nothing.
  const [more, setMore] = useState<number | null>(null);
  const edges = useScrollEdges<HTMLElement>((e) => setMore(e.below ? e.progress : null));
  return (
    <div className="scroll-frame" data-grow={grow || undefined} hidden={hidden}>
      <Tag className={`${className} scroller`} aria-label={label} hidden={hidden} ref={edges}>
        {children}
      </Tag>
      {more !== null && (
        <div className="scroll-rail" aria-hidden="true">
          <span className="scroll-readout">{`▼ ${more}%`}</span>
        </div>
      )}
    </div>
  );
}
