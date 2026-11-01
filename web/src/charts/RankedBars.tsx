import { useState } from "react";
import { column, layout, motion, space } from "../generated/layout";
import { ChartTooltip } from "./ChartTooltip";
import { HiddenTable } from "./HiddenTable";
import { NoData } from "./NoData";
import { paintVar, type Paint } from "./paint";
import { finite, linear, niceTicks, plainFormat, type Format } from "./scale";
import { ScaleTicks } from "./ScaleTicks";
import { useChartEntry } from "./useChartEntry";
import { useSize } from "./useSize";
import "./charts.css";

/** Label, bar and value per row in the caller's order; the block never sorts. */
export function RankedBars({ label, rows, format = plainFormat }: {
  label: string;
  rows: { label: string; value: number; paint?: Paint }[];
  format?: Format;
}) {
  const [ref, { width: whole }] = useSize<HTMLDivElement>();
  const [hover, setHover] = useState<{ i: number; x: number; y: number } | null>(null);
  // The root is what gets measured (it always renders); the track is what is left of its width between the label and value columns.
  const width = Math.max(whole - layout.chartLabelW - layout.chartValueW - 2 * space.buttonGap, 0);
  const drawn = width > 0 && rows.length > 0;
  const entering = useChartEntry(drawn);

  const max = Math.max(0, ...rows.map((r) => finite(r.value)));
  const ticks = niceTicks(max);
  const top = ticks[ticks.length - 1] ?? 0;
  const pct = linear(0, top, 0, 100);
  const table: [string, string][] = rows.map((r) => [r.label, format(finite(r.value))]);

  return (
    <div ref={ref} className="chart-rank" role="group" aria-label={label}>
      {rows.length === 0 && <NoData />}
      {rows.map((r, i) => (
        <div
          key={i}
          className="chart-rank-row"
          onMouseEnter={(e) => {
            const row = e.currentTarget;
            setHover({ i, x: row.offsetWidth / 2, y: row.offsetTop + row.offsetHeight });
          }}
          onMouseLeave={() => setHover((h) => (h?.i === i ? null : h))}
        >
          <span className="chart-rank-label" aria-hidden="true">{r.label}</span>
          <span className="chart-rank-track">
            {drawn && (
              <span
                className="chart-rank-bar"
                data-enter={entering || undefined}
                style={{
                  "--i": Math.min(i, motion.staggerCap),
                  width: `${finite(r.value) > 0 ? pct(finite(r.value)) : 0}%`,
                  background: paintVar(r.paint ?? { series: 0 }),
                } as React.CSSProperties}
              />
            )}
          </span>
          <span className="chart-rank-value" aria-hidden="true">{format(finite(r.value))}</span>
        </div>
      ))}
      {rows.length > 0 && (
        <div className="chart-rank-row chart-rank-scale">
          <span />
          <div className="chart-rank-track">
            {width > 0 && (
              <svg width={width} height={column.tickLen + layout.captionGap + layout.cardPadding} aria-hidden="true">
                <ScaleTicks width={width} ticks={ticks} max={top} format={format} labels={width >= layout.chartTiny} />
              </svg>
            )}
          </div>
          <span />
        </div>
      )}
      {hover && rows[hover.i] && (
        <ChartTooltip x={hover.x} y={hover.y} lines={[rows[hover.i]!.label, format(finite(rows[hover.i]!.value))]} />
      )}
      <HiddenTable caption={label} rows={table} />
    </div>
  );
}
