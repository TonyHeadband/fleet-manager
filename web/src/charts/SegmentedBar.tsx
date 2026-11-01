import { useState } from "react";
import { column, layout, motion } from "../generated/layout";
import { ChartTooltip } from "./ChartTooltip";
import { HiddenTable } from "./HiddenTable";
import { Legend } from "./Legend";
import { NoData } from "./NoData";
import { paintVar, type Paint } from "./paint";
import { finite, limitTicks, linear, plainFormat, type Format } from "./scale";
import { ScaleTicks } from "./ScaleTicks";
import { useChartEntry } from "./useChartEntry";
import { useSize } from "./useSize";
import "./charts.css";

/** Parts of a whole end to end on a scale up to `max`; a total over `max` clamps the drawing, never the readings. */
export function SegmentedBar({ label, segments, max, format = plainFormat }: {
  label: string;
  segments: { label: string; value: number; paint: Paint }[];
  max: number;
  format?: Format;
}) {
  const [ref, { width }] = useSize<HTMLDivElement>();
  const [hover, setHover] = useState<number | null>(null);
  const total = segments.reduce((sum, s) => sum + Math.max(finite(s.value), 0), 0);
  const empty = total <= 0;
  const drawn = width > 0 && !empty;
  const entering = useChartEntry(drawn);

  const limit = Math.max(finite(max), 0);
  const ticks = limitTicks(limit);
  const x = linear(0, limit, 0, width);
  const barH = layout.barH;
  const bottom = barH + layout.captionGap;
  const rows: [string, string][] = segments.map((s) => [s.label, format(finite(s.value))]);

  let at = 0;
  const marks = segments.map((s) => {
    const from = Math.min(at, limit);
    at += Math.max(finite(s.value), 0);
    const to = Math.min(at, limit);
    return { from: limit > 0 ? x(from) : 0, to: limit > 0 ? x(to) : 0 };
  });
  const used = marks.length ? marks[marks.length - 1]!.to : 0;

  return (
    <div ref={ref} className="chart-parts" role="group" aria-label={label}>
      {empty && <NoData />}
      {width > 0 && (
        <svg width={width} height={bottom + column.tickLen + layout.captionGap + layout.cardPadding} aria-hidden="true">
          {empty ? (
            <rect className="chart-outline" x={0} y={0} width={width} height={barH} />
          ) : (
            <>
              {used < width && <rect className="chart-outline" x={used} y={0} width={width - used} height={barH} />}
              {segments.map((s, i) => (
                <rect
                  key={i}
                  className="chart-seg"
                  data-enter={entering ? "" : undefined}
                  style={{ "--i": Math.min(i, motion.staggerCap) } as React.CSSProperties}
                  x={marks[i]!.from}
                  y={0}
                  width={marks[i]!.to - marks[i]!.from}
                  height={barH}
                  fill={paintVar(s.paint)}
                />
              ))}
              {segments.map((_, i) => (
                <rect
                  key={`hit-${i}`}
                  className="chart-seg-hit"
                  x={marks[i]!.from}
                  y={0}
                  width={marks[i]!.to - marks[i]!.from}
                  height={bottom}
                  onMouseEnter={() => setHover(i)}
                  onMouseLeave={() => setHover((h) => (h === i ? null : h))}
                />
              ))}
            </>
          )}
          <g transform={`translate(0 ${bottom})`}>
            <ScaleTicks width={width} ticks={ticks} max={limit} format={format} labels={width >= layout.chartTiny} />
          </g>
        </svg>
      )}
      {drawn && segments.length >= 2 && width >= layout.chartCompact && (
        <Legend items={segments.map((s) => ({ label: `${s.label} ${format(finite(s.value))}`, paint: s.paint }))} />
      )}
      {hover !== null && segments[hover] && (
        <ChartTooltip
          x={(marks[hover]!.from + marks[hover]!.to) / 2}
          y={bottom}
          lines={[segments[hover]!.label, format(finite(segments[hover]!.value))]}
        />
      )}
      <HiddenTable caption={label} rows={rows} />
    </div>
  );
}
