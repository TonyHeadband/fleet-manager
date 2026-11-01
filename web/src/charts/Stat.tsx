import { useEffect, useRef, useState } from "react";
import { motion } from "../generated/layout";
import { HiddenTable } from "./HiddenTable";
import { plainFormat, type Format } from "./scale";
import { Sparkline } from "./Sparkline";
import { useChartEntry } from "./useChartEntry";
import "./charts.css";

type Delta = { value: number; period: string; tone: "good" | "bad" | "neutral" };

const DELTA_COLOR = { good: "var(--tone-done)", bad: "var(--tone-danger)", neutral: "var(--text-low)" } as const;

/** Counts up from 0 over the entry window; once the entry is over, a new value shows at once. */
function useStepUp(value: number, entering: boolean): number {
  const [shown, setShown] = useState(0);
  const latest = useRef(value);
  latest.current = value;
  useEffect(() => {
    if (!entering) return;
    let start: number | undefined;
    let frame = requestAnimationFrame(function step(ts) {
      start ??= ts;
      const t = Math.min(1, (ts - start) / motion.wipe);
      setShown(latest.current * t);
      if (t < 1) frame = requestAnimationFrame(step);
    });
    return () => cancelAnimationFrame(frame);
  }, [entering]);
  return entering ? shown : value;
}

export function Stat({ label, value, format = plainFormat, delta, spark }: {
  label: string;
  value: number;
  format?: Format;
  delta?: Delta;
  spark?: number[];
}) {
  const safe = Number.isFinite(value) ? value : 0;
  const entering = useChartEntry(true);
  const shown = useStepUp(safe, entering);
  const deltaText = delta ? `${format(Math.abs(delta.value))} / ${delta.period}` : "";
  const rows: [string, string][] = [["Value", format(safe)]];
  if (delta) rows.push(["Change", `${delta.value < 0 ? "-" : ""}${deltaText}`]);

  return (
    <div className="chart-stat" role="group" aria-label={label}>
      <div className="chart-stat-head">
        <span className="chart-stat-value" aria-hidden="true">{format(shown)}</span>
        {delta && (
          <span className="chart-stat-delta" aria-hidden="true" style={{ color: DELTA_COLOR[delta.tone] }}>
            {delta.value < 0 ? "▼" : "▲"} {deltaText}
          </span>
        )}
      </div>
      {spark && spark.length > 0 && <Sparkline values={spark} />}
      <HiddenTable caption={label} rows={rows} />
    </div>
  );
}
