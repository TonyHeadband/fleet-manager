import { layout } from "../generated/layout";
import { paintVar, type Paint } from "./paint";
import { linear } from "./scale";
import { useChartEntry } from "./useChartEntry";
import { useSize } from "./useSize";
import "./charts.css";

/** A line with no axes: the block's trend at a glance. A single point is a marker alone. */
export function Sparkline({ values, paint = { series: 0 } }: { values: number[]; paint?: Paint }) {
  const [ref, { width, height: measured }] = useSize<HTMLDivElement>();
  // The region's basis is the token height; it grows into a taller container and shrinks in a short one (Stat's cell).
  const height = measured > 0 ? Math.max(measured, layout.markerH) : layout.chartSpark;
  const ys = values.map((v) => (Number.isFinite(v) ? v : 0));
  const drawn = width > 0 && ys.length > 0;
  const entering = useChartEntry(drawn);
  const color = paintVar(paint);

  let body = null;
  if (drawn) {
    const inset = layout.chartLine;
    const lo = Math.min(...ys);
    const hi = Math.max(...ys);
    const y = hi === lo ? () => height / 2 : linear(lo, hi, height - inset, inset);
    const x = ys.length === 1 ? () => width - layout.markerW : linear(0, ys.length - 1, 0, width);
    const last = ys.length - 1;
    body = (
      <>
        <line className="chart-baseline" x1={0} x2={width} y1={height} y2={height} />
        {ys.length > 1 && (
          <polyline
            className="chart-spark-line"
            data-enter={entering || undefined}
            fill="none"
            stroke={color}
            style={{ strokeWidth: "var(--layout-chart-line)" }}
            points={ys.map((v, i) => `${x(i)},${y(v)}`).join(" ")}
          />
        )}
        <rect
          className="chart-marker"
          data-enter={entering || undefined}
          x={x(last) - (ys.length > 1 ? layout.markerW : 0)}
          y={y(ys[last] ?? 0) - layout.markerH / 2}
          width={layout.markerW}
          height={layout.markerH}
          fill={color}
        />
      </>
    );
  }

  return (
    <div className="chart-spark">
      <div ref={ref} className="chart-plot" style={{ flexBasis: layout.chartSpark, minHeight: layout.markerH }}>
        <svg width={width} height={height} aria-hidden="true">
          {body}
        </svg>
      </div>
    </div>
  );
}
