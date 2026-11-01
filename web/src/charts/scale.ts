export type Format = (n: number) => string;

export const finite = (n: number) => (Number.isFinite(n) ? n : 0);

export function plainFormat(n: number): string {
  if (!Number.isFinite(n)) return "0";
  return n.toLocaleString(undefined, { maximumFractionDigits: Number.isInteger(n) ? 0 : 1 });
}

/** Zero-based ticks ending at a nice step multiple at or above `max`. */
export function niceTicks(max: number, count = 4): number[] {
  if (!Number.isFinite(max) || max <= 0 || count < 1) return [0];
  const raw = max / count;
  const mag = 10 ** Math.floor(Math.log10(raw));
  const step = ([1, 2, 5, 10].find((m) => m * mag >= raw) ?? 10) * mag;
  const ticks: number[] = [];
  for (let i = 0; ; i++) {
    const t = Math.round(i * step * 1e9) / 1e9;
    ticks.push(t);
    if (t >= max) break;
  }
  return ticks;
}

/** A flat domain maps everything to `r0`, so a zero max never divides by zero. */
export function linear(d0: number, d1: number, r0: number, r1: number): (v: number) => number {
  const span = d1 - d0;
  if (!span || !Number.isFinite(span)) return () => r0;
  return (v) => r0 + ((v - d0) / span) * (r1 - r0);
}

/** Nice ticks that stop at `max` itself, for scales whose end is the limit (a capacity), not a rounded-up axis. */
export function limitTicks(max: number, count = 4): number[] {
  if (!Number.isFinite(max) || max <= 0) return [0];
  return [...niceTicks(max, count).filter((t) => t < max), max];
}
