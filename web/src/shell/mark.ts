// The mark in its 64-unit square: the craft (nose, tail, C) inside a radar ring. The ring is scaled up from Lite's
// `fui::radar_mark` to leave space around the craft, and may spill past the square; drawings keep overflow visible.

export const HUB = { x: 32, y: 34 };
export const RING_SCALE = 1.3;
export const CIRCLE_R = 23 * RING_SCALE;
export const DOT_RING_R = 26.5 * RING_SCALE;
export const TICK_IN_R = 24.5 * RING_SCALE;
export const DOT_SIZE = 0.35;
/** About one dot every 3 units, rounded to a multiple of 4 so a dot sits at the end of each cardinal tick. */
export const DOTS = 4 * Math.round((2 * Math.PI * 26.5) / 3 / 4);
/** One turn of the boot mark's ring. */
export const RING_TURN_MS = 15000;
/** The drawer's watermark turns slower: there it is background. */
export const WATERMARK_TURN_MS = 30000;
/** Its dots: twice the boot mark's, or they vanish at the drawer's scale. */
export const WATERMARK_DOT_SIZE = 2 * DOT_SIZE;

export const NOSE = "32,11 35.5,30 28.5,30";
export const TAIL = "29.5,41.5 34.5,41.5 32,46.5";
export const C_PATH = "M17 51 A15 18 0 0 1 47 51 A15 15.5 0 0 0 17 51 Z";
/** The C's centre line, left to right: what reveals it. */
export const C_SPINE = "M17 51 A15 16.75 0 0 1 47 51";

export type Point3 = [number, number, number];

export interface Tilt {
  x: number;
  y: number;
  spin: number;
}

export interface Projected {
  x: number;
  y: number;
  z: number;
  f: number;
}

export const clamp = (v: number) => Math.max(0, Math.min(1, v));
const lerp = (a: number, b: number, p: number) => a + (b - a) * p;

/** `n` evenly spaced angles, 0 = up, clockwise. */
export function ringAngles(n: number): number[] {
  return Array.from({ length: n }, (_, i) => ringAngle(i, n));
}

/** The `i`th of `n` evenly spaced angles, 0 = up, clockwise; `i = n` comes back round to the first. */
export function ringAngle(i: number, n: number): number {
  return (2 * Math.PI * i) / n;
}

/** The cardinal points, where the ticks end in triangles instead of dots. */
export const CARDINALS = ringAngles(4);
/** The ring's dots, less the four at the cardinals. */
export const DOT_ANGLES = ringAngles(DOTS).filter((_, i) => i % (DOTS / 4) !== 0);

// A cardinal triangle, sized by the ring's dots so a mark with bigger dots gets bigger triangles.
const MARKER_LENGTH = 4;
const MARKER_HALF_WIDTH = 2.3;

/**
 * The triangle at angle `a`, hub-relative: its base on the dot ring (radius `r`), its apex pointing at the hub. Apex
 * first, then the base's two corners.
 */
export function marker(a: number, dotSize: number, r = DOT_RING_R): [Point3, Point3, Point3] {
  const apex = onRing(r - MARKER_LENGTH * dotSize, a);
  const [bx, by] = onRing(r, a);
  // Along the ring at `a`.
  const [tx, ty] = [Math.cos(a), Math.sin(a)];
  const w = MARKER_HALF_WIDTH * dotSize;
  return [apex, [bx - tx * w, by - ty * w, 0], [bx + tx * w, by + ty * w, 0]];
}

/** How far the ring reaches from the hub: the dot ring's outer edge (the triangles' bases sit on it). */
export const ringReach = (dotSize: number) => DOT_RING_R + dotSize;

/** How far the ring reaches below the 64-unit square, as a fraction of it. */
export const ringSpillBottom = (dotSize: number) => (HUB.y + ringReach(dotSize) - 64) / 64;

/** The point `r` from the hub at angle `a` (0 = up, clockwise), relative to the hub. */
export function onRing(r: number, a: number): Point3 {
  return [r * Math.sin(a), -r * Math.cos(a), 0];
}

/**
 * A hub-relative point turned by the ring's spin about its own axis, then tilted about X and Y, in perspective. The
 * spin comes first so that when the tilt reaches zero the same spin simply carries on face-on.
 */
export function project([x, y, z]: Point3, tilt: Tilt): Projected {
  let c = Math.cos(tilt.spin);
  let s = Math.sin(tilt.spin);
  [x, y] = [x * c - y * s, x * s + y * c];
  c = Math.cos(tilt.x);
  s = Math.sin(tilt.x);
  [y, z] = [y * c - z * s, y * s + z * c];
  c = Math.cos(tilt.y);
  s = Math.sin(tilt.y);
  [x, z] = [x * c + z * s, -x * s + z * c];
  const f = 140 / (140 + z);
  return { x: HUB.x + x * f, y: HUB.y + y * f, z, f };
}

/** The far half of a ring of radius `r` dims to a quarter: what makes the tumbling ring read as a globe. */
export function depthAlpha(z: number, r: number): number {
  return r < 0.01 ? 1 : lerp(1, 0.25, clamp((z / r + 1) / 2));
}
