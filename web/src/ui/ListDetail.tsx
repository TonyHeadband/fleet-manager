import type { ReactNode } from "react";
import { ScrollArea } from "./ScrollArea";

interface Props {
  /** Which list width token: the Tasks view's, or Management's tabs'. */
  width: "task" | "management";
  /** Names the list column for assistive tech. */
  listLabel: string;
  /** Names the detail pane. */
  detailLabel: string;
  list: ReactNode;
  /** Pinned under the list while it scrolls (egui's `Panel::bottom` in the list panel). */
  footer?: ReactNode;
  detail: ReactNode;
}

/** egui's `Panel::left` beside a `CentralPanel`: a fixed-width list and the selected item's detail, each scrolling. */
export function ListDetail({ width, listLabel, detailLabel, list, footer, detail }: Props) {
  return (
    <div className="list-detail" data-width={width}>
      <ScrollArea as="section" className="list-detail-list" label={listLabel}>
        {list}
        {footer && <div className="list-detail-footer">{footer}</div>}
      </ScrollArea>
      <ScrollArea as="section" className="list-detail-detail" label={detailLabel} grow>
        {detail}
      </ScrollArea>
    </div>
  );
}
