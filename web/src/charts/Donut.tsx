import { useState } from "react";
import { column, layout, motion } from "../generated/layout";
import { ChartTooltip } from "./ChartTooltip";
import { HiddenTable } from "./HiddenTable";
import { Legend } from "./Legend";
import { NoData } from "./NoData";
import { paintVar, type Paint } from "./paint";
import { finite, plainFormat, type Format } from "./scale";
import { useChartEntry } from "./useChartEntry";
import { useSize } from "./useSize";
import "./charts.css";

const RING = layout.cardPadding;
/** Radius of the ring's centre line in a square of `size`, leaving room for the quarter ticks. */
const midRadius = (size: number) => size / 2 - column.tickLen - RING / 2;
const MIN = layout.chartPlot + 2 * column.tickLen;
const WHOLE = 1 - 1e-6;

/** Point on the mid-ring at `share` of a turn clockwise from 12 o'clock. */
function at(share: number, size: number): [number, number] {
  const C = size / 2;
  const a = share * 2 * Math.PI;
  const mid = midRadius(size);
  return [C + mid * Math.sin(a), C - mid * Math.cos(a)];
}

const percent = (share: number) => `${Math.round(share * 100)}%`;

export function Donut({ label, slices, center, format = plainFormat }: {
  label: string;
  slices: { label: string; value: number; paint: Paint }[];
  center?: string;
  format?: Format;
}) {
  const [ref, { width, height: measured }] = useSize<HTMLDivElement>();
  const [hover, setHover] = useState<{ i: number; x: number } | null>(null);
  const total = slices.reduce((sum, s) => sum + Math.max(finite(s.value), 0), 0);
  const empty = total <= 0;
  // Square, as tall as the container gives; never wider than the width left beside the legend.
  const legend = !empty && slices.length >= 2 && width >= layout.chartCompact;
  const SIZE = Math.max(MIN, Math.min(measured, legend ? width / 2 : width));
  const C = SIZE / 2;
  const MID = midRadius(SIZE);
  const drawn = width > 0 && !empty;
  const entering = useChartEntry(drawn);

  const share = (i: number) => (total > 0 ? Math.max(finite(slices[i]!.value), 0) / total : 0);
  const rows: [string, string][] = slices.map((s) => [s.label, format(finite(s.value))]);
  let from = 0;
  const arcs = slices.map((_, i) => {
    const s = share(i);
    const arc = { from, share: s };
    from += s;
    return arc;
  });
  const quarters: [number, number, number, number][] = [
    [C, 0, C, column.tickLen],
    [SIZE, C, SIZE - column.tickLen, C],
    [C, SIZE, C, SIZE - column.tickLen],
    [0, C, column.tickLen, C],
  ];

  return (
    <div ref={ref} className="chart-parts chart-donut" role="group" aria-label={label}>
      {empty && <NoData />}
      {width > 0 && (
        <svg width={SIZE} height={SIZE} aria-hidden="true">
          {empty ? (
            <circle className="chart-outline" cx={C} cy={C} r={MID} strokeWidth={RING} />
          ) : (
            arcs.map((a, i) => {
              if (a.share <= 0) return null;
              const common = {
                className: "chart-donut-arc",
                "data-share": a.share,
                "data-enter": entering ? "sweep" : undefined,
                style: { "--i": Math.min(i, motion.staggerCap) } as React.CSSProperties,
                stroke: paintVar(slices[i]!.paint),
                strokeWidth: RING,
                pathLength: 100,
                strokeDasharray: 100,
                pointerEvents: "stroke" as const,
                onMouseEnter: () => {
                  const [mx] = at(a.from + a.share / 2, SIZE);
                  setHover({ i, x: mx });
                },
                onMouseLeave: () => setHover((h) => (h?.i === i ? null : h)),
              };
              if (a.share >= WHOLE) return <circle key={i} {...common} cx={C} cy={C} r={MID} transform={`rotate(-90 ${C} ${C})`} />;
              const [x0, y0] = at(a.from, SIZE);
              const [x1, y1] = at(a.from + a.share, SIZE);
              return <path key={i} {...common} d={`M ${x0} ${y0} A ${MID} ${MID} 0 ${a.share > 0.5 ? 1 : 0} 1 ${x1} ${y1}`} />;
            })
          )}
          <g className="chart-donut-ticks">
            {quarters.map(([x1, y1, x2, y2], i) => (
              <line key={i} className="chart-tick" x1={x1} y1={y1} x2={x2} y2={y2} />
            ))}
          </g>
          {center && <text className="chart-donut-center" x={C} y={C} textAnchor="middle" dominantBaseline="central">{center}</text>}
        </svg>
      )}
      {drawn && legend && (
        <Legend items={slices.map((s, i) => ({ label: `${s.label} ${percent(share(i))}`, paint: s.paint }))} />
      )}
      {hover && slices[hover.i] && (
        <ChartTooltip
          x={hover.x}
          y={SIZE}
          lines={[slices[hover.i]!.label, `${format(finite(slices[hover.i]!.value))} · ${percent(share(hover.i))}`]}
        />
      )}
      <HiddenTable caption={label} rows={rows} />
    </div>
  );
}
