import type { ReactNode } from "react";

interface Props {
  /** Which list width token: the Tasks view's, or Management's tabs'. */
  width: "task" | "management";
  /** Names the list column for assistive tech. */
  listLabel: string;
  /** Names the detail pane. */
  detailLabel: string;
  list: ReactNode;
  detail: ReactNode;
}

/** egui's `Panel::left` beside a `CentralPanel`: a fixed-width list and the selected item's detail, each scrolling. */
export function ListDetail({ width, listLabel, detailLabel, list, detail }: Props) {
  return (
    <div className="list-detail" data-width={width}>
      <div className="list-detail-list" aria-label={listLabel}>
        {list}
      </div>
      <section className="list-detail-detail" aria-label={detailLabel}>
        {detail}
      </section>
    </div>
  );
}
