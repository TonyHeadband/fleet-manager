import { RING_TURN_MS, type Tilt } from "../shell/mark";

// The app's boot animation as a function of the time since it began (specs/2026-10-30-boot-animation-design.md in
// the internal repo). The ring arrives tumbling and lands face-on; the craft is revealed after it: nose, tail, then
// the C with a spark on its tip.

/** The assembly is complete: only the ring's spin moves after this. */
export const BOOT_MS = 3050;
export const CRAFT_DELAY_MS = 400;

const RING = [0, 1600] as const;
const MERIDIANS = [900, 1700] as const;
// Craft phases, from the craft's start.
const NOSE = [0, 700] as const;
const TAIL = [700, 1300] as const;
const C = [1300, 2300] as const;
const SPARK_OUT = [2300, 2650] as const;

export interface BootFrame {
  /** The ring's size, 0 to 1. */
  scale: number;
  tilt: Tilt;
  /** The two extra great circles' opacity. */
  meridians: number;
  /** Reveal progress, 0 to 1. */
  nose: number;
  tail: number;
  c: number;
  /** The spark's visibility: 0 before the C starts and once it has faded. */
  spark: number;
}

const clamp = (v: number) => Math.max(0, Math.min(1, v));
const phase = (t: number, [a, b]: readonly [number, number]) => clamp((t - a) / (b - a));
export const easeOut = (p: number) => 1 - Math.pow(1 - p, 3);
export const easeInOut = (p: number) => (p < 0.5 ? 4 * p * p * p : 1 - Math.pow(-2 * p + 2, 3) / 2);

export function bootFrame(ms: number): BootFrame {
  const landed = easeOut(phase(ms, RING));
  const tc = ms - CRAFT_DELAY_MS;
  const c = easeInOut(phase(tc, C));
  return {
    scale: landed,
    // One constant spin from the first frame; only the tilt eases, so landing changes nothing about the spin.
    tilt: { x: (1 - landed) * 0.9 * Math.PI, y: (1 - landed) * 0.6 * Math.PI, spin: (2 * Math.PI * ms) / RING_TURN_MS },
    meridians: 1 - easeInOut(phase(ms, MERIDIANS)),
    nose: easeInOut(phase(tc, NOSE)),
    tail: easeInOut(phase(tc, TAIL)),
    c,
    spark: c > 0 ? 1 - phase(tc, SPARK_OUT) : 0,
  };
}
