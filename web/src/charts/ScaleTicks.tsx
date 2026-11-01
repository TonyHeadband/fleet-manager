import "./charts.css";
import { column, layout } from "../generated/layout";
import { linear, type Format } from "./scale";

const LABEL_Y = column.tickLen + layout.captionGap;

/** An accent baseline along y=0 with a tick (and optionally a label) per value; the block places the group. */
export function ScaleTicks({ width, ticks, max, format, labels }: { width: number; ticks: number[]; max: number; format: Format; labels: boolean }) {
  const x = linear(0, max, 0, width);
  return (
    <g className="chart-scale">
      <line className="chart-baseline" x1={0} x2={width} y1={0} y2={0} />
      {ticks.map((t) => (
        <g key={t}>
          <line className="chart-tick" x1={x(t)} x2={x(t)} y1={0} y2={column.tickLen} />
          {labels && (
            <text className="chart-tick-label" x={x(t)} y={LABEL_Y} textAnchor="middle" dominantBaseline="hanging">
              {format(t)}
            </text>
          )}
        </g>
      ))}
    </g>
  );
}
