import { useRef, useState } from "react";
import { column, layout, motion } from "../generated/layout";
import { ChartTooltip } from "./ChartTooltip";
import { HiddenTable } from "./HiddenTable";
import { Legend } from "./Legend";
import { NoData } from "./NoData";
import { paintVar, type Paint } from "./paint";
import { ScaleTicks } from "./ScaleTicks";
import { finite, linear, niceTicks, plainFormat, type Format } from "./scale";
import { clockFormat, timeTicks } from "./timeAxis";
import { useChartEntry } from "./useChartEntry";
import { useSize } from "./useSize";
import "./charts.css";

type Point = { t: number; v: number | null };
type Series = { label: string; paint: Paint; points: Point[] };
type Dot = { t: number; v: number };

/** Runs of consecutive drawable points; a null (or non-finite time) ends a run. */
function runs(points: Point[]): Dot[][] {
  const out: Dot[][] = [];
  let cur: Dot[] = [];
  for (const p of points) {
    if (p.v === null || !Number.isFinite(p.t)) {
      if (cur.length) out.push(cur);
      cur = [];
    } else cur.push({ t: p.t, v: finite(p.v) });
  }
  if (cur.length) out.push(cur);
  return out;
}

/** Trims a run to [from, to], interpolating where a segment crosses an edge so the line still reaches it. */
function clipRun(run: Dot[], from: number, to: number): Dot[] {
  if (run.length === 1) return run[0]!.t >= from && run[0]!.t <= to ? run : [];
  const out: Dot[] = [];
  const at = (a: Dot, b: Dot, t: number): Dot => ({ t, v: a.v + ((b.v - a.v) * (t - a.t)) / (b.t - a.t || 1) });
  for (let i = 1; i < run.length; i++) {
    const a = run[i - 1]!;
    const b = run[i]!;
    if (Math.max(a.t, b.t) < from || Math.min(a.t, b.t) > to) continue;
    const start = a.t < from ? at(a, b, from) : a;
    const end = b.t > to ? at(a, b, to) : b;
    const last = out[out.length - 1];
    if (!last || last.t !== start.t || last.v !== start.v) out.push(start);
    out.push(end);
  }
  return out;
}

export function TimeSeries({ label, from, to, series, formatX = clockFormat, formatY = plainFormat }: {
  label: string;
  from: number;
  to: number;
  series: Series[];
  formatX?: (t: number) => string;
  formatY?: Format;
}) {
  const [ref, { width }] = useSize<HTMLDivElement>();
  const svgRef = useRef<SVGSVGElement>(null);
  const [hoverT, setHoverT] = useState<number | null>(null);
  const hasData = series.some((s) => s.points.length > 0);
  const drawn = width > 0 && hasData;
  const entering = useChartEntry(drawn);

  const left = layout.chartValueW;
  const top = layout.captionGap;
  const plotW = Math.max(width - left, 0);
  const plotH = layout.chartPlot;
  const span = Number.isFinite(to - from) ? Math.max(to - from, 0) : 0;
  const x = linear(from, from + span, 0, plotW);
  const max = Math.max(0, ...series.flatMap((s) => s.points.map((p) => (p.v === null ? 0 : finite(p.v)))));
  const yTicks = niceTicks(max);
  const yTop = yTicks[yTicks.length - 1] ?? 0;
  const y = linear(0, yTop, plotH, 0);
  const drawY = (v: number) => y(Math.max(v, 0));
  const showLabels = width >= layout.chartTiny;
  const xTicks = timeTicks(span, plotW);
  const showLegend = series.length >= 2 && width >= layout.chartCompact;

  const rows: [string, string][] = series.flatMap((s) =>
    s.points.map((p): [string, string] => [`${s.label} · ${formatX(p.t)}`, p.v === null ? "—" : formatY(finite(p.v))]),
  );

  const move = (e: React.MouseEvent) => {
    const px = e.clientX - (svgRef.current?.getBoundingClientRect().left ?? 0) - left;
    setHoverT(from + (Math.min(Math.max(px, 0), plotW) / (plotW || 1)) * span);
  };

  const valueAt = (s: Series, t: number): string => {
    let hit: Point | undefined;
    for (const p of s.points) if (p.t <= t) hit = p;
    return !hit || hit.v === null ? "—" : formatY(finite(hit.v));
  };

  return (
    <div ref={ref} className="chart-ts" role="group" aria-label={label}>
      {!hasData && <NoData />}
      {drawn && (
        <>
          <svg ref={svgRef} width={width} height={top + plotH + column.tickLen + layout.captionGap + layout.markerH} aria-hidden="true">
            <g transform={`translate(${left} ${top})`}>
              {yTicks.map((t, i) => (
                <g key={t}>
                  {i > 0 && <line className="chart-grid" x1={0} x2={plotW} y1={y(t)} y2={y(t)} />}
                  {showLabels && (
                    <text className="chart-tick-label chart-ytick" x={-layout.captionGap} y={y(t)} textAnchor="end" dominantBaseline="middle">
                      {formatY(t)}
                    </text>
                  )}
                </g>
              ))}
              <g transform={`translate(0 ${plotH})`}>
                <ScaleTicks width={plotW} ticks={xTicks} max={span} format={(t) => formatX(from + t)} labels={showLabels} />
              </g>
              {series.map((s, si) => {
                const color = paintVar(s.paint);
                const enter = { "data-enter": entering || undefined, style: { "--i": Math.min(si, motion.staggerCap) } as React.CSSProperties };
                return (
                  <g key={si}>
                    {runs(s.points).map((run, ri) => {
                      const c = clipRun(run, from, to);
                      if (c.length === 1)
                        return (
                          <rect
                            key={ri}
                            className="chart-marker"
                            {...enter}
                            x={x(c[0]!.t) - layout.markerW / 2}
                            y={drawY(c[0]!.v) - layout.markerH / 2}
                            width={layout.markerW}
                            height={layout.markerH}
                            fill={color}
                          />
                        );
                      if (!c.length) return null;
                      return (
                        <polyline
                          key={ri}
                          className="chart-ts-line"
                          {...enter}
                          fill="none"
                          stroke={color}
                          style={{ ...enter.style, strokeWidth: "var(--layout-chart-line)" }}
                          points={c.map((d) => `${x(d.t)},${drawY(d.v)}`).join(" ")}
                        />
                      );
                    })}
                  </g>
                );
              })}
              {hoverT !== null && <line className="chart-cross" x1={x(hoverT)} x2={x(hoverT)} y1={0} y2={plotH} />}
              <rect className="chart-ts-hit" x={0} y={0} width={plotW} height={plotH} onMouseMove={move} onMouseLeave={() => setHoverT(null)} />
            </g>
          </svg>
          {showLegend && <Legend items={series.map((s) => ({ label: s.label, paint: s.paint }))} />}
          {hoverT !== null && (
            <ChartTooltip
              x={left + x(hoverT)}
              y={top + plotH}
              lines={[formatX(hoverT), ...series.map((s) => `${s.label}: ${valueAt(s, hoverT)}`)]}
            />
          )}
        </>
      )}
      <HiddenTable caption={label} rows={rows} />
    </div>
  );
}
