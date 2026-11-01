import "./charts.css";
import { linear, type Format } from "./scale";

const TICK = 4;
const LABEL_Y = 14;

/** An accent baseline along y=0 with a tick (and optionally a label) per value; the block places the group. */
export function ScaleTicks({ width, ticks, max, format, labels }: { width: number; ticks: number[]; max: number; format: Format; labels: boolean }) {
  const x = linear(0, max, 0, width);
  return (
    <g className="chart-scale">
      <line className="chart-baseline" x1={0} x2={width} y1={0} y2={0} />
      {ticks.map((t) => (
        <g key={t}>
          <line className="chart-tick" x1={x(t)} x2={x(t)} y1={0} y2={TICK} />
          {labels && (
            <text className="chart-tick-label" x={x(t)} y={LABEL_Y} textAnchor="middle">
              {format(t)}
            </text>
          )}
        </g>
      ))}
    </g>
  );
}
