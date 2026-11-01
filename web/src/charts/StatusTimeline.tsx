import { useState } from "react";
import { column, layout, motion, space } from "../generated/layout";
import { ChartTooltip } from "./ChartTooltip";
import { HiddenTable } from "./HiddenTable";
import { Legend } from "./Legend";
import { NoData } from "./NoData";
import { type Tone } from "./paint";
import { ScaleTicks } from "./ScaleTicks";
import { finite, linear } from "./scale";
import { clockFormat, timeTicks } from "./timeAxis";
import { useChartEntry } from "./useChartEntry";
import { useSize } from "./useSize";
import "./charts.css";

type Band = { from: number; to: number | null; tone: Tone; label: string };
type Lane = { label: string; bands: Band[] };
type Shown = Band & { start: number; end: number; open: boolean };

/** Bands cut to [from, to]; a band wholly outside, or of no length inside, is dropped. An open band ends at `to`. */
function visible(bands: Band[], from: number, to: number): Shown[] {
  const out: Shown[] = [];
  for (const b of bands) {
    if (!Number.isFinite(b.from)) continue;
    const start = Math.max(b.from, from);
    const end = Math.min(b.to === null || !Number.isFinite(b.to) ? to : b.to, to);
    if (end > start) out.push({ ...b, start, end, open: b.to === null });
  }
  return out;
}

export function StatusTimeline({ label, from, to, lanes, formatX = clockFormat }: {
  label: string;
  from: number;
  to: number;
  lanes: Lane[];
  formatX?: (t: number) => string;
}) {
  const [ref, { width: whole }] = useSize<HTMLDivElement>();
  const [hover, setHover] = useState<{ lane: number; band: number; x: number; y: number } | null>(null);
  const gap = space.buttonGap;
  const labelW = layout.chartLabelW + gap;
  const width = Math.max(whole - labelW, 0);
  const drawn = width > 0 && lanes.length > 0;
  const entering = useChartEntry(drawn);

  const span = Number.isFinite(to - from) ? Math.max(to - from, 0) : 0;
  const x = linear(from, from + span, 0, width);
  const shown = lanes.map((l) => visible(l.bands, finite(from), finite(from) + span));
  const showLabels = whole >= layout.chartTiny;

  const states: { label: string; tone: Tone }[] = [];
  for (const b of shown.flat()) if (!states.some((s) => s.tone === b.tone)) states.push({ label: b.label, tone: b.tone });
  const showLegend = states.length >= 2 && whole >= layout.chartCompact;

  const range = (b: Shown) => `${formatX(b.start)} – ${b.open ? "now" : formatX(b.end)}`;
  const rows: [string, string][] = lanes.flatMap((l, li) =>
    shown[li]!.map((b): [string, string] => [`${l.label} · ${b.label}`, range(b)]),
  );

  return (
    <div ref={ref} className="chart-tl" role="group" aria-label={label}>
      {lanes.length === 0 && <NoData />}
      {lanes.map((l, li) => (
        <div key={li} className="chart-tl-row">
          <span className="chart-tl-label" aria-hidden="true">{l.label}</span>
          <span className="chart-tl-lane">
            {drawn &&
              shown[li]!.map((b, bi) => {
                const left = x(b.start);
                return (
                  <span
                    key={bi}
                    className="chart-tl-band"
                    style={{ left, width: Math.max(x(b.end) - left, 0) }}
                    onMouseEnter={(e) => {
                      const row = e.currentTarget.closest<HTMLElement>(".chart-tl-row");
                      setHover({ lane: li, band: bi, x: labelW + left + (x(b.end) - left) / 2, y: (row?.offsetTop ?? 0) + (row?.offsetHeight ?? 0) });
                    }}
                    onMouseLeave={() => setHover((h) => (h?.lane === li && h.band === bi ? null : h))}
                  >
                    <span
                      className="chart-tl-fill"
                      data-enter={entering || undefined}
                      style={{ "--i": Math.min(li, motion.staggerCap), background: `var(--tone-${b.tone})` } as React.CSSProperties}
                    />
                  </span>
                );
              })}
          </span>
        </div>
      ))}
      {drawn && (
        <div className="chart-tl-row chart-tl-axis">
          <span />
          <svg width={width} height={column.tickLen + layout.captionGap + layout.cardPadding} aria-hidden="true">
            <ScaleTicks width={width} ticks={timeTicks(span, width)} max={span} format={(t) => formatX(from + t)} labels={showLabels} />
          </svg>
        </div>
      )}
      {drawn && showLegend && <Legend items={states.map((s) => ({ label: s.label, paint: { tone: s.tone } }))} />}
      {hover && shown[hover.lane]?.[hover.band] && (
        <ChartTooltip
          x={hover.x}
          y={hover.y}
          lines={[lanes[hover.lane]!.label, shown[hover.lane]![hover.band]!.label, range(shown[hover.lane]![hover.band]!)]}
        />
      )}
      <HiddenTable caption={label} rows={rows} />
    </div>
  );
}
