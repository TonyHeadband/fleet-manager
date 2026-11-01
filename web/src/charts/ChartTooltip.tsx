import { useLayoutEffect, useRef, useState } from "react";
import "./charts.css";

const GAP = 12;

/** Opens below (x, y) in the block's coordinates, kept inside the block's width. */
export function ChartTooltip({ x, y, lines }: { x: number; y: number; lines: string[] }) {
  const ref = useRef<HTMLDivElement>(null);
  const [left, setLeft] = useState(x);
  useLayoutEffect(() => {
    const el = ref.current;
    const room = el?.parentElement?.clientWidth ?? 0;
    if (!el || !room) return setLeft(x);
    setLeft(Math.min(Math.max(x - el.offsetWidth / 2, 0), Math.max(room - el.offsetWidth, 0)));
  }, [x, lines]);
  return (
    <div ref={ref} role="tooltip" className="fui-tooltip chart-tooltip" style={{ left, top: y + GAP }}>
      {lines.map((l, i) => (
        <div key={i}>{l}</div>
      ))}
    </div>
  );
}
