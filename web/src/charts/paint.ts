export type Tone = "neutral" | "active" | "attention" | "done" | "muted" | "caution" | "danger";
export type Paint = { tone: Tone } | { series: 0 | 1 | 2 | 3 | 4 | 5 | "other" };

/** Status uses a tone, a category a series slot; "other" is the muted tone so the rest never competes. */
export function paintVar(p: Paint): string {
  if ("tone" in p) return `var(--tone-${p.tone})`;
  return p.series === "other" ? "var(--tone-muted)" : `var(--series-${p.series})`;
}
