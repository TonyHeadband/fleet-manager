import { useState } from "react";
import { column, layout, motion } from "../generated/layout";
import { ChartTooltip } from "./ChartTooltip";
import { HiddenTable } from "./HiddenTable";
import { NoData } from "./NoData";
import { paintVar, type Paint } from "./paint";
import { linear, plainFormat, type Format } from "./scale";
import { useChartEntry } from "./useChartEntry";
import { useSize } from "./useSize";
import "./charts.css";

/** The 1px line a zero (or negative) bar leaves on the baseline: half the data line's width. */
const SLIVER = layout.chartLine / 2;

const finite = (n: number) => (Number.isFinite(n) ? n : 0);

export function ColumnBars({ label, bars, format = plainFormat }: {
  label: string;
  bars: { label: string; value: number; paint: Paint }[];
  format?: Format;
}) {
  const [ref, { width }] = useSize<HTMLDivElement>();
  const [hover, setHover] = useState<number | null>(null);
  const drawn = width > 0 && bars.length > 0;
  const entering = useChartEntry(drawn);

  const plot = layout.chartPlot;
  const band = bars.length ? width / bars.length : 0;
  const barW = Math.max(band - column.gap, SLIVER);
  const max = Math.max(0, ...bars.map((b) => finite(b.value)));
  const height = linear(0, max, 0, plot - layout.cardPadding);
  const showValues = width >= layout.chartTiny;
  const rows: [string, string][] = bars.map((b) => [b.label, format(finite(b.value))]);

  return (
    <div ref={ref} className="chart-bars" role="group" aria-label={label}>
      {bars.length === 0 && <NoData />}
      {drawn && (
        <>
          <svg width={width} height={plot + column.tickLen} aria-hidden="true">
            {bars.map((b, i) => {
              const h = max > 0 && b.value > 0 ? height(b.value) : SLIVER;
              const x = i * band + (band - barW) / 2;
              const cx = i * band + band / 2;
              return (
                <g key={i}>
                  <rect
                    className="chart-col-bar"
                    data-enter={entering ? "grow" : undefined}
                    style={{ "--i": Math.min(i, motion.staggerCap) } as React.CSSProperties}
                    x={x}
                    y={plot - h}
                    width={barW}
                    height={h}
                    fill={paintVar(b.paint)}
                  />
                  {showValues && (
                    <text className="chart-col-value" x={cx} y={plot - h - layout.captionGap} textAnchor="middle">
                      {format(finite(b.value))}
                    </text>
                  )}
                  <line className="chart-tick" x1={cx} x2={cx} y1={plot} y2={plot + column.tickLen} />
                  <rect
                    className="chart-col-hit"
                    x={i * band}
                    y={0}
                    width={band}
                    height={plot}
                    onMouseEnter={() => setHover(i)}
                    onMouseLeave={() => setHover((h) => (h === i ? null : h))}
                  />
                </g>
              );
            })}
            <line className="chart-baseline" x1={0} x2={width} y1={plot} y2={plot} />
          </svg>
          <div className="chart-col-labels" style={{ gridTemplateColumns: `repeat(${bars.length}, minmax(0, 1fr))` }} aria-hidden="true">
            {bars.map((b, i) => (
              <span key={i} className="chart-col-label">{b.label}</span>
            ))}
          </div>
          {hover !== null && bars[hover] && (
            <ChartTooltip x={hover * band + band / 2} y={plot} lines={[bars[hover].label, format(finite(bars[hover].value))]} />
          )}
        </>
      )}
      <HiddenTable caption={label} rows={rows} />
    </div>
  );
}
