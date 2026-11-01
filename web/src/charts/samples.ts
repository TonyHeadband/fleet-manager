import type { Tone } from "./paint";

/** A fixed "now" (epoch seconds) so previews and tests are deterministic. */
export const SAMPLE_NOW = 1_793_545_200;
const HOUR = 3600;

const usd = (n: number) => `$${n.toFixed(2)}`;
const gb = (n: number) => `${n} GB`;
const tokens = (n: number) => (n >= 1000 ? `${Math.round(n / 1000)}k` : String(n));

const band = (from: number, to: number | null, tone: Tone, label: string) => ({ from: SAMPLE_NOW - from * 60, to: to === null ? null : SAMPLE_NOW - to * 60, tone, label });

/** Each sample is shaped like the data the block's first widget will feed it; status uses tones, categories series. */
export const SAMPLES = {
  agentsLive: {
    label: "Agents live",
    value: 11,
    delta: { value: 3, period: "1h", tone: "good" as const },
    spark: [6, 7, 5, 8, 8, 9, 9, 10, 9, 11, 12, 11],
  },
  agentsByStatus: {
    label: "Agents by status",
    bars: [
      { label: "Working", value: 5, paint: { tone: "active" as const } },
      { label: "Needs you", value: 2, paint: { tone: "attention" as const } },
      { label: "Review", value: 3, paint: { tone: "done" as const } },
      { label: "Talking", value: 1, paint: { tone: "caution" as const } },
      { label: "Queued", value: 0, paint: { tone: "muted" as const } },
    ],
  },
  storageByKind: {
    label: "Storage",
    max: 50,
    format: gb,
    segments: [
      { label: "worktrees", value: 16.2, paint: { series: 0 as const } },
      { label: "build", value: 9.1, paint: { series: 1 as const } },
      { label: "logs", value: 3.4, paint: { series: 2 as const } },
    ],
  },
  costByProduct: {
    label: "Cost by product",
    center: "$41",
    format: usd,
    slices: [
      { label: "fleet-manager", value: 20.6, paint: { series: 0 as const } },
      { label: "portal", value: 11.5, paint: { series: 1 as const } },
      { label: "docs", value: 6.6, paint: { series: 2 as const } },
      { label: "other", value: 2.5, paint: { series: "other" as const } },
    ],
  },
  tokensPerHour: {
    label: "Tokens per hour",
    from: SAMPLE_NOW - 24 * HOUR,
    to: SAMPLE_NOW,
    formatY: tokens,
    series: [
      {
        label: "input",
        paint: { series: 0 as const },
        points: [8, 10, 12, 20, 70, 120, 105, 150, 140, 165, 160, 135, 148, 118, 125, 155, 172, 168, 162, 150, 140, 158, 165, 160, 162].map((k, i) => ({ t: SAMPLE_NOW - (24 - i) * HOUR, v: k * 1000 })),
      },
      {
        label: "output",
        paint: { series: 1 as const },
        points: [2, 3, 4, 6, 22, 38, 33, 48, 45, 52, 50, 42, 46, 38, 40, 48, 54, 52, 50, 46, 44, 50, 53, 52, 54].map((k, i) => ({ t: SAMPLE_NOW - (24 - i) * HOUR, v: k * 1000 })),
      },
    ],
  },
  topTasksByCost: {
    label: "Top tasks by cost",
    format: usd,
    rows: [
      { label: "ghost-tasks", value: 9.8 },
      { label: "chat-pane", value: 7.6 },
      { label: "diff-pane", value: 4.02 },
      { label: "avatars", value: 2.15 },
      { label: "port-race", value: 0.88 },
    ],
  },
  dailyBudget: {
    label: "Daily budget",
    value: 41.2,
    max: 60,
    format: usd,
    marks: [{ at: 48, label: "warn 48", tone: "caution" as const }],
  },
  agentActivity: {
    label: "Agent activity",
    from: SAMPLE_NOW - 2 * HOUR,
    to: SAMPLE_NOW,
    lanes: [
      { label: "orch chat-pane", bands: [band(120, 87, "active", "working"), band(87, 80, "attention", "needs you"), band(80, 31, "active", "working"), band(31, 0, "done", "review")] },
      { label: "spec · tests", bands: [band(100, 45, "active", "working"), band(45, 0, "done", "review")] },
      { label: "orch diff-pane", bands: [band(120, 105, "active", "working"), band(105, 80, "caution", "talking"), band(80, 0, "active", "working")] },
      { label: "solo avatars", bands: [band(70, 12, "active", "working"), band(12, null, "attention", "needs you")] },
    ],
  },
};
