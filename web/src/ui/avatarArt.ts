/**
 * Generated avatars: a picture computed from an id, like an identicon in the FUI look. A persona is a placeholder
 * figure filled with scan lines; a specialist is a set of circuit traces. Pure and DOM-free; `Avatar.tsx` draws it.
 *
 * The geometry is ported from the owner-approved prototype (fleet-manager-internal
 * specs/assets/2026-10-31-avatar-prototype.html) in the same PRNG call order, so the pictures match what was tuned.
 * The output for a seed is a compatibility promise: changing anything here changes every avatar in the fleet.
 */

/** A supervisor is the persona figure with a square head: the same scan art, a different silhouette. */
export type AvatarKind = "persona" | "specialist" | "supervisor";
/** `main` paints at --avatar-l, `dim` at --avatar-l2. */
export type Ink = "main" | "dim";
export type AvatarShape =
  | { type: "line"; x1: number; y1: number; x2: number; y2: number; ink: Ink; opacity: number; width: number }
  | { type: "polyline"; points: [number, number][]; ink: Ink; opacity: number; width: number };
export interface AvatarArt {
  kind: AvatarKind;
  hue: number;
  shapes: AvatarShape[];
}

export const AVATAR_W = 30,
  AVATAR_H = 40;
/** The persona silhouette: a head circle and a shoulder circle cut by the bottom edge. */
export const FIGURE = { head: { cx: 15, cy: 14, r: 6.6 }, shoulders: { cx: 15, cy: 39, r: 12.5 }, outline: 0.75 } as const;

type Pt = [number, number];

/** 32-bit FNV-1a over the UTF-8 bytes. */
function fnv(s: string): number {
  let h = 2166136261;
  for (const b of new TextEncoder().encode(s)) {
    h ^= b;
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

/** mulberry32 seeded with the seed's hash. */
function rng(seed: string): () => number {
  let a = fnv(seed);
  return () => {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export function avatar(kind: AvatarKind, seed: string): AvatarArt {
  const hue = Math.floor((fnv("hue:" + seed) / 4294967296) * 360);
  const r = rng(seed);
  return { kind, hue, shapes: kind === "specialist" ? traces(r) : scan(r) };
}

/** Each persona's supervisor has its own stable picture, distinct from the persona's own. */
export const supervisorSeed = (persona: string) => `supervisor:${persona}`;

const SCAN_WIDTHS = [0.5, 0.75, 1.1];
const SCAN_OPACITIES = [1, 0.7, 0.45];

/** Rows every 1.6 units, each one weight, walked in runs and gaps; the component clips them to the figure. */
function scan(r: () => number): AvatarShape[] {
  const shapes: AvatarShape[] = [];
  for (let y = 1; y < AVATAR_H; y += 1.6) {
    const width = SCAN_WIDTHS[Math.floor(r() * 3)]!;
    let x = 0;
    while (x < AVATAR_W) {
      const run = 2 + r() * 9;
      const gap = r() < 0.6 ? 0.8 + r() * 3 : 0;
      if (r() < 0.75) {
        const ink: Ink = r() < 0.2 ? "dim" : "main";
        const opacity = SCAN_OPACITIES[Math.floor(r() * 3)]!;
        shapes.push({ type: "line", x1: x, y1: y, x2: Math.min(AVATAR_W, x + run), y2: y, ink, opacity, width });
      }
      x += run + gap + 0.8;
    }
  }
  return shapes;
}

const STEP = 2;
const TRACE_WIDTH = 0.75;
const MARGIN = 1.5;
const BUSES = 4;
const TRIES = 40;
/** Occupancy cell size, and the padding each sample marks around itself so buses keep a gap. */
const CELL = STEP / 2;
const DIRS: Pt[] = [
  [1, 0],
  [0, 1],
  [-1, 0],
  [0, -1],
];

/** The occupancy cells a set of lines covers, padded. */
function cells(lines: Pt[][]): Set<string> {
  const key = (x: number, y: number) => `${Math.floor(x / CELL)},${Math.floor(y / CELL)}`;
  const out = new Set<string>();
  for (const p of lines)
    for (let i = 0; i < p.length - 1; i++) {
      const [x1, y1] = p[i]!,
        [x2, y2] = p[i + 1]!;
      const n = Math.ceil(Math.hypot(x2 - x1, y2 - y1) / (CELL / 2)) || 1;
      for (let t = 0; t <= n; t++) {
        const x = x1 + ((x2 - x1) * t) / n,
          y = y1 + ((y2 - y1) * t) / n;
        for (const dx of [-CELL * 0.6, 0, CELL * 0.6]) for (const dy of [-CELL * 0.6, 0, CELL * 0.6]) out.add(key(x + dx, y + dy));
      }
    }
  return out;
}

/** A parallel copy of a path at distance `d`, with mitred corners. */
function offset(p: Pt[], d: number): Pt[] {
  const normal = (a: Pt, b: Pt): Pt => {
    const dx = b[0] - a[0],
      dy = b[1] - a[1],
      l = Math.hypot(dx, dy);
    return [-dy / l, dx / l];
  };
  return p.map((v, i) => {
    const n1 = i > 0 ? normal(p[i - 1]!, v) : null;
    const n2 = i < p.length - 1 ? normal(v, p[i + 1]!) : null;
    if (!n1) return [v[0] + d * n2![0], v[1] + d * n2![1]];
    if (!n2) return [v[0] + d * n1[0], v[1] + d * n1[1]];
    const k = 1 + n1[0] * n2[0] + n1[1] * n2[1];
    return [v[0] + (d * (n1[0] + n2[0])) / k, v[1] + (d * (n1[1] + n2[1])) / k];
  });
}

/** One bus of `n` parallel lines: a lead path from an edge with right-angle turns, or null when it folds onto itself. */
function bus(r: () => number, n: number): Pt[][] | null {
  const hw = ((n - 1) * STEP) / 2,
    lo = MARGIN + hw,
    hiX = AVATAR_W - MARGIN - hw,
    hiY = AVATAR_H - MARGIN - hw;
  const side = Math.floor(r() * 4);
  const snap = (v: number, a: number, b: number) => Math.max(a, Math.min(b, a + Math.round((v - a) / STEP) * STEP));
  let x: number, y: number, d: number;
  if (side === 0) [x, y, d] = [0, snap(lo + r() * (hiY - lo), lo, hiY), 0];
  else if (side === 1) [x, y, d] = [snap(lo + r() * (hiX - lo), lo, hiX), 0, 1];
  else if (side === 2) [x, y, d] = [AVATAR_W, snap(lo + r() * (hiY - lo), lo, hiY), 2];
  else [x, y, d] = [snap(lo + r() * (hiX - lo), lo, hiX), AVATAR_H, 3];
  const lead: Pt[] = [[x, y]];
  const segs = 3 + Math.floor(r() * 3),
    minL = hw * 2 + STEP * 2;
  for (let i = 0; i < segs; i++) {
    const [dx, dy] = DIRS[d]!;
    const len = minL + Math.floor(r() * 5) * STEP;
    // The last segment may run off the board instead.
    const toEdge = i === segs - 1 && r() < 0.4;
    let nx: number, ny: number;
    if (toEdge) {
      nx = dx > 0 ? AVATAR_W : dx < 0 ? 0 : x;
      ny = dy > 0 ? AVATAR_H : dy < 0 ? 0 : y;
    } else {
      nx = Math.max(lo, Math.min(hiX, x + dx * len));
      ny = Math.max(lo, Math.min(hiY, y + dy * len));
    }
    if (Math.hypot(nx - x, ny - y) < minL * 0.6) break;
    x = nx;
    y = ny;
    lead.push([x, y]);
    d = (d + (r() < 0.5 ? 1 : 3)) % 4;
  }
  if (lead.length < 2) return null;
  const e = hw + STEP * 0.5;
  const box = (a: Pt, b: Pt): [number, number, number, number] => [Math.min(a[0], b[0]) - e, Math.min(a[1], b[1]) - e, Math.max(a[0], b[0]) + e, Math.max(a[1], b[1]) + e];
  for (let i = 0; i < lead.length - 1; i++)
    for (let j = i + 2; j < lead.length - 1; j++) {
      const a = box(lead[i]!, lead[i + 1]!),
        b = box(lead[j]!, lead[j + 1]!);
      if (a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]) return null;
    }
  return Array.from({ length: n }, (_, j) => offset(lead, j * STEP - hw));
}

/** Up to four buses placed by rejection so none crosses another; the first in main ink, the last one dimmed. */
function traces(r: () => number): AvatarShape[] {
  const shapes: AvatarShape[] = [];
  const occupied = new Set<string>();
  let placed = 0;
  for (let tries = 0; tries < TRIES && placed < BUSES; tries++) {
    const lines = bus(r, 2 + Math.floor(r() * 3));
    if (!lines) continue;
    const covered = cells(lines);
    if ([...covered].some((k) => occupied.has(k))) continue;
    covered.forEach((k) => occupied.add(k));
    const ink: Ink = placed === 0 ? "main" : "dim";
    const opacity = placed === BUSES - 1 ? 0.45 : 1;
    for (const points of lines) shapes.push({ type: "polyline", points, ink, opacity, width: TRACE_WIDTH });
    placed++;
  }
  return shapes;
}
