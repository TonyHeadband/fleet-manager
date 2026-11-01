import { useState } from "react";
import { column, layout } from "../generated/layout";
import { ChartTooltip } from "./ChartTooltip";
import { HiddenTable } from "./HiddenTable";
import { paintVar, type Tone } from "./paint";
import { finite, limitTicks, linear, plainFormat, type Format } from "./scale";
import { ScaleTicks } from "./ScaleTicks";
import { useChartEntry } from "./useChartEntry";
import { useSize } from "./useSize";
import "./charts.css";

/** One fill against `max`; a value over `max` fills to the end and still reads true. */
export function Meter({ label, value, max, marks = [], format = plainFormat }: {
  label: string;
  value: number;
  max: number;
  marks?: { at: number; label: string; tone: Tone }[];
  format?: Format;
}) {
  const [ref, { width }] = useSize<HTMLDivElement>();
  const [hover, setHover] = useState<{ kind: "fill" } | { mark: number } | null>(null);
  const drawn = width > 0;
  const entering = useChartEntry(drawn);

  const limit = Math.max(finite(max), 0);
  const ticks = limitTicks(limit);
  const x = linear(0, limit, 0, width);
  const clamp = (v: number) => (limit > 0 ? Math.min(Math.max(x(finite(v)), 0), width) : 0);
  const inset = layout.chartLine / 2;
  const top = marks.length ? layout.cardPadding : 0;
  const barY = top + layout.captionGap;
  const bottom = barY + layout.barH;
  const fill = clamp(value);
  const tip = !hover ? null : "mark" in hover
    ? marks[hover.mark] && { x: clamp(marks[hover.mark]!.at), lines: [marks[hover.mark]!.label, format(finite(marks[hover.mark]!.at))] }
    : { x: fill / 2, lines: [label, format(finite(value))] };
  const rows: [string, string][] = [[label, format(finite(value))], ...marks.map((m): [string, string] => [m.label, format(finite(m.at))])];

  return (
    <div ref={ref} className="chart-parts" role="group" aria-label={label}>
      {drawn && (
        <svg width={width} height={bottom + layout.captionGap + column.tickLen + layout.captionGap + layout.cardPadding} aria-hidden="true">
          <rect className="chart-outline" x={0} y={barY} width={width} height={layout.barH} />
          <rect
            className="chart-meter-fill"
            data-enter={entering ? "" : undefined}
            x={inset}
            y={barY + inset}
            width={Math.max(fill - 2 * inset, 0)}
            height={layout.barH - 2 * inset}
            fill={paintVar({ series: 0 })}
          />
          <rect
            className="chart-seg-hit"
            x={0}
            y={barY}
            width={width}
            height={layout.barH}
            onMouseEnter={() => setHover({ kind: "fill" })}
            onMouseLeave={() => setHover(null)}
          />
          {marks.map((m, i) => {
            const mx = clamp(m.at);
            return (
              <g key={i}>
                <line className="chart-meter-mark" x1={mx} x2={mx} y1={top} y2={bottom + layout.captionGap} stroke={paintVar({ tone: m.tone })} />
                <text className="chart-mark-label" x={mx} y={top - inset} textAnchor="middle">{m.label}</text>
                <rect
                  className="chart-seg-hit"
                  x={mx - layout.cardPadding / 2}
                  y={top}
                  width={layout.cardPadding}
                  height={bottom + layout.captionGap - top}
                  onMouseEnter={() => setHover({ mark: i })}
                  onMouseLeave={() => setHover(null)}
                />
              </g>
            );
          })}
          <g transform={`translate(0 ${bottom + layout.captionGap})`}>
            <ScaleTicks width={width} ticks={ticks} max={limit} format={format} labels={width >= layout.chartTiny} />
          </g>
        </svg>
      )}
      {tip && <ChartTooltip x={tip.x} y={bottom} lines={tip.lines} />}
      <HiddenTable caption={label} rows={rows} />
    </div>
  );
}
