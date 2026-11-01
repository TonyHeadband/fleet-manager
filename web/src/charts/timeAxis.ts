import { layout } from "../generated/layout";

const pad2 = (n: number) => String(n).padStart(2, "0");

/** Default time label: local HH:MM from epoch seconds. */
export const clockFormat = (t: number) => {
  const d = new Date(t * 1000);
  return `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
};

/** Tick offsets (seconds from the range start) across a span: four steps when wide, two when narrow, none for an empty range. */
export function timeTicks(span: number, plotW: number): number[] {
  const steps = span ? (plotW >= 2 * layout.chartCompact ? 4 : 2) : 0;
  return Array.from({ length: steps + 1 }, (_, i) => (steps ? (span * i) / steps : 0));
}
