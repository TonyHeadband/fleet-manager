import { describe, expect, it } from "vitest";
import { AVATAR_H, AVATAR_W, avatar, type AvatarArt, type AvatarKind, type AvatarShape } from "./avatarArt";

// The generator behind every persona and specialist picture. The same seed must draw the same picture forever, in
// both themes and on every machine, so these tests pin both its invariants and, for a few seeds, its exact output.

const KINDS: AvatarKind[] = ["persona", "specialist"];
const PLAIN = Array.from({ length: 200 }, (_, i) => `p${i}`);
const SEEDS = [...PLAIN, "é-ünïcode-🛰", "x".repeat(200)];
const EPS = 1e-9;

type Seg = [[number, number], [number, number]];

function points(s: AvatarShape): [number, number][] {
  return s.type === "line" ? [[s.x1, s.y1], [s.x2, s.y2]] : s.points;
}

function segments(s: AvatarShape): Seg[] {
  const p = points(s);
  return p.slice(1).map((q, i) => [p[i]!, q]);
}

/** Whether two closed segments share any point (crossing, touching or overlapping). */
function meet([a, b]: Seg, [c, d]: Seg): boolean {
  const orient = (p: [number, number], q: [number, number], r: [number, number]) => {
    const v = (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
    return Math.abs(v) < EPS ? 0 : Math.sign(v);
  };
  const on = (p: [number, number], q: [number, number], r: [number, number]) =>
    Math.min(p[0], q[0]) - EPS <= r[0] && r[0] <= Math.max(p[0], q[0]) + EPS && Math.min(p[1], q[1]) - EPS <= r[1] && r[1] <= Math.max(p[1], q[1]) + EPS;
  const o1 = orient(a, b, c), o2 = orient(a, b, d), o3 = orient(c, d, a), o4 = orient(c, d, b);
  if (o1 !== o2 && o3 !== o4) return true;
  return (o1 === 0 && on(a, b, c)) || (o2 === 0 && on(a, b, d)) || (o3 === 0 && on(c, d, a)) || (o4 === 0 && on(c, d, b));
}

function fingerprint(art: AvatarArt): string {
  const text = JSON.stringify(art, (_, v) => (typeof v === "number" ? Math.round(v * 1e4) / 1e4 : v));
  let h = 2166136261;
  for (const b of new TextEncoder().encode(text)) {
    h ^= b;
    h = Math.imul(h, 16777619);
  }
  return (h >>> 0).toString(16);
}

describe("avatar", () => {
  it("the_same_seed_draws_the_same_picture", () => {
    for (const k of KINDS) for (const s of SEEDS) expect(avatar(k, s)).toEqual(avatar(k, s));
  });

  it("different_seeds_draw_different_pictures", () => {
    for (const k of KINDS) {
      const distinct = new Set(PLAIN.map((s) => JSON.stringify(avatar(k, s).shapes)));
      expect(distinct.size, k).toBeGreaterThanOrEqual(195);
    }
  });

  it("the_hue_is_its_own_hash", () => {
    for (const s of SEEDS) {
      const hue = avatar("persona", s).hue;
      expect(avatar("specialist", s).hue).toBe(hue);
      expect(hue).toBeGreaterThanOrEqual(0);
      expect(hue).toBeLessThan(360);
    }
  });

  it("every_point_is_finite_and_in_the_box", () => {
    for (const k of KINDS)
      for (const s of SEEDS)
        for (const shape of avatar(k, s).shapes)
          for (const [x, y] of points(shape)) {
            expect(Number.isFinite(x) && Number.isFinite(y), `${k} ${s}`).toBe(true);
            expect(x >= -EPS && x <= AVATAR_W + EPS && y >= -EPS && y <= AVATAR_H + EPS, `${k} ${s} (${x}, ${y})`).toBe(true);
          }
  });

  it("persona_scan_rows_are_1_6_apart_with_three_weights", () => {
    for (const s of SEEDS) {
      const rows = new Map<number, Set<number>>();
      for (const shape of avatar("persona", s).shapes) {
        expect(shape.type).toBe("line");
        if (shape.type !== "line") continue;
        expect(Math.abs(shape.y1 - shape.y2)).toBeLessThan(EPS);
        const k = (shape.y1 - 1) / 1.6;
        expect(Math.abs(k - Math.round(k)), `${s} y=${shape.y1}`).toBeLessThan(EPS);
        expect([0.5, 0.75, 1.1]).toContain(shape.width);
        expect([1, 0.7, 0.45]).toContain(shape.opacity);
        const row = rows.get(Math.round(k)) ?? new Set<number>();
        row.add(shape.width);
        rows.set(Math.round(k), row);
      }
      for (const widths of rows.values()) expect(widths.size, s).toBe(1);
    }
  });

  it("specialist_traces_turn_only_at_right_angles", () => {
    for (const s of SEEDS)
      for (const shape of avatar("specialist", s).shapes) {
        expect(shape.type).toBe("polyline");
        expect(shape.width).toBe(0.75);
        expect([1, 0.45]).toContain(shape.opacity);
        const segs = segments(shape);
        for (const [[x1, y1], [x2, y2]] of segs) expect(Math.abs(x1 - x2) < EPS || Math.abs(y1 - y2) < EPS, s).toBe(true);
        for (let i = 1; i < segs.length; i++) {
          const [[ax, ay], [bx, by]] = segs[i - 1]!;
          const [[cx, cy], [dx, dy]] = segs[i]!;
          expect(Math.abs((bx - ax) * (dx - cx) + (by - ay) * (dy - cy)), s).toBeLessThan(EPS);
        }
      }
  });

  it("specialist_traces_never_cross", () => {
    for (const s of SEEDS) {
      const shapes = avatar("specialist", s).shapes;
      expect(shapes.length, s).toBeGreaterThan(0);
      for (let i = 0; i < shapes.length; i++)
        for (let j = i + 1; j < shapes.length; j++)
          for (const a of segments(shapes[i]!)) for (const b of segments(shapes[j]!)) expect(meet(a, b), `${s}: ${i} × ${j}`).toBe(false);
    }
  });

  it("only_the_first_bus_is_main_ink", () => {
    for (const s of SEEDS) {
      const shapes = avatar("specialist", s).shapes;
      const firstDim = shapes.findIndex((x) => x.ink === "dim");
      expect(shapes[0]!.ink, s).toBe("main");
      if (firstDim >= 0) expect(shapes.slice(firstDim).every((x) => x.ink === "dim"), s).toBe(true);
    }
  });

  // Recorded from the first passing run, which matched the prototype exactly for these seeds.
  it("golden_seeds_keep_their_picture", () => {
    const got = Object.fromEntries(
      ["claude", "ui-to-tauri", "landing"].flatMap((s) => KINDS.map((k) => [`${k}:${s}`, fingerprint(avatar(k, s))])),
    );
    expect(got).toEqual({
      "persona:claude": "4ea2f869",
      "specialist:claude": "57733276",
      "persona:ui-to-tauri": "4fff59cf",
      "specialist:ui-to-tauri": "ebce427c",
      "persona:landing": "4967b986",
      "specialist:landing": "3f83ecf1",
    });
  });
});
