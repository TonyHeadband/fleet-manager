import type { ReactNode } from "react";
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
 * A scroller in a frame: the scroller marks the edges it goes on past (`data-more`), and the frame is where an
 * indicator for them is drawn.
 */
export function ScrollArea({ as: Tag = "div", className, grow, label, hidden, children }: Props) {
  const edges = useScrollEdges<HTMLElement>();
  return (
    <div className="scroll-frame" data-grow={grow || undefined} hidden={hidden}>
      <Tag className={`${className} scroller`} aria-label={label} hidden={hidden} ref={edges}>
        {children}
      </Tag>
    </div>
  );
}
