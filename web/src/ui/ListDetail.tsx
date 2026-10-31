import type { ReactNode } from "react";
import { useScrollFade } from "./scrollFade";

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
  const listFade = useScrollFade<HTMLElement>();
  const detailFade = useScrollFade<HTMLElement>();
  return (
    <div className="list-detail" data-width={width}>
      <section className="list-detail-list scroll-fade" aria-label={listLabel} ref={listFade}>
        {list}
        {footer && <div className="list-detail-footer">{footer}</div>}
      </section>
      <section className="list-detail-detail scroll-fade" aria-label={detailLabel} ref={detailFade}>
        {detail}
      </section>
    </div>
  );
}
