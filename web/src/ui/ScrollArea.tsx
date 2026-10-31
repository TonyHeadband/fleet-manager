import type { ReactNode } from "react";
import { useScrollFade } from "./scrollFade";

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
 * A scroller in a frame: the scroller's edges fade where it goes on, and the frame lays the accent tint over them,
 * which a mask on the scroller alone cannot colour.
 */
export function ScrollArea({ as: Tag = "div", className, grow, label, hidden, children }: Props) {
  const fade = useScrollFade<HTMLElement>();
  return (
    <div className="scroll-frame" data-grow={grow || undefined} hidden={hidden}>
      <Tag className={`${className} scroll-fade`} aria-label={label} hidden={hidden} ref={fade}>
        {children}
      </Tag>
    </div>
  );
}
