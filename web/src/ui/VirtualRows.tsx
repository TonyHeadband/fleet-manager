import { useLayoutEffect, useRef, useState, type ReactNode } from "react";

/** When the list is not laid out yet (a hidden page, a test), how many rows it shows. */
const FALLBACK_ROWS = 40;
/** Rows drawn above and below the visible ones, so a quick scroll does not show a gap. */
const OVERSCAN = 10;

/**
 * egui's `ScrollArea::show_rows` with `stick_to_bottom`: only the visible rows are in the page, each `rowHeight` tall,
 * and the list follows new rows while it is at the bottom; scrolled up, it stays put.
 */
export function VirtualRows({
  count,
  rowHeight,
  row,
  className = "",
}: {
  count: number;
  rowHeight: number;
  row: (i: number) => ReactNode;
  className?: string;
}) {
  const box = useRef<HTMLDivElement>(null);
  const [top, setTop] = useState(0);
  const [height, setHeight] = useState(0);
  const stuck = useRef(true);

  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    setHeight(el.clientHeight);
    if (stuck.current) el.scrollTop = el.scrollHeight;
  }, [count]);
  // A resized window shows more or fewer rows: measured again, and kept at the bottom when it was there.
  useLayoutEffect(() => {
    const el = box.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => {
      setHeight(el.clientHeight);
      if (stuck.current) el.scrollTop = el.scrollHeight;
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const visible = height > 0 ? Math.ceil(height / rowHeight) : FALLBACK_ROWS;
  const first = stuck.current ? Math.max(0, count - visible) : Math.floor(top / rowHeight);
  const from = Math.max(0, first - OVERSCAN);
  const to = Math.min(count, first + visible + OVERSCAN);
  return (
    <div
      ref={box}
      className={`virtual-rows ${className}`}
      onScroll={(e) => {
        const el = e.currentTarget;
        stuck.current = el.scrollTop + el.clientHeight >= el.scrollHeight - 1;
        setTop(el.scrollTop);
        setHeight(el.clientHeight);
      }}
    >
      <div style={{ height: count * rowHeight, position: "relative" }}>
        <div style={{ position: "absolute", top: from * rowHeight, left: 0, right: 0 }}>
          {Array.from({ length: to - from }, (_, k) => (
            <div key={from + k} style={{ height: rowHeight }}>
              {row(from + k)}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
