import { useEffect, useId, useRef } from "react";
import {
  C_PATH,
  C_SPINE,
  CIRCLE_R,
  depthAlpha,
  DOT_RING_R,
  DOT_SIZE,
  DOTS,
  HUB,
  NOSE,
  onRing,
  project,
  RING_SPILL_BOTTOM,
  ringAngle,
  TAIL,
  TICK_IN_R,
  type Point3,
  type Tilt,
} from "../shell/mark";
import { BOOT_MS, bootFrame, type BootFrame } from "./bootTimeline";

/** The 64-unit square at 320 px. */
export const BOOT_UNIT_PX = 5;

/** The bar spans the dot ring. */
export const bootBarWidth = () => 2 * DOT_RING_R * BOOT_UNIT_PX;

const SEGMENTS = 96;
const MERIDIAN_SEGMENTS = 72;
// Room for 350 ms of particles at 144 Hz.
const TRAIL = 64;
const TRAIL_MS = 350;
// The ring spills past the square; the box grows by the overflow so nothing overlaps the bar.
const OUTER = DOT_RING_R + DOT_SIZE;
const PAD_TOP = (OUTER - HUB.y) * BOOT_UNIT_PX;
const PAD_BOTTOM = RING_SPILL_BOTTOM * 64 * BOOT_UNIT_PX;

// The two great circles at right angles to the ring: its plane turned onto YZ and onto XZ.
const MERIDIAN_PLANES = [([x, y, z]: Point3): Point3 => [z, y, x], ([x, y, z]: Point3): Point3 => [x, z, y]];

/** The C's spine sampled with the length along it: the half ellipse from (17, 51) over to (47, 51). */
const SPINE = (() => {
  const n = 64;
  let len = 0;
  let prev = { x: 17, y: 51 };
  return Array.from({ length: n + 1 }, (_, i) => {
    const t = (Math.PI * i) / n;
    const at = { x: 32 - 15 * Math.cos(t), y: 51 - 16.75 * Math.sin(t) };
    len += Math.hypot(at.x - prev.x, at.y - prev.y);
    prev = at;
    return { ...at, len };
  });
})();

/** The point a fraction `p` of the way along the C's spine, by length: where its reveal has reached. */
function spinePoint(p: number): [number, number] {
  const want = p * SPINE[SPINE.length - 1]!.len;
  const i = Math.max(
    1,
    SPINE.findIndex((s) => s.len >= want),
  );
  const a = SPINE[i - 1]!;
  const b = SPINE[i] ?? a;
  const k = b.len > a.len ? (want - a.len) / (b.len - a.len) : 0;
  return [a.x + (b.x - a.x) * k, a.y + (b.y - a.y) * k];
}

type Attrs = Record<string, number | string>;
const set = (el: Element, attrs: Attrs) => {
  for (const k in attrs) el.setAttribute(k, String(attrs[k]));
};

interface Particle {
  x: number;
  y: number;
  born: number;
  vx: number;
  vy: number;
}

/** Every node a frame touches, found once. */
function nodesOf(svg: SVGSVGElement) {
  const all = (sel: string) => Array.from(svg.querySelectorAll(sel));
  const one = (sel: string) => svg.querySelector(sel)!;
  return {
    segments: all(".boot-seg"),
    dots: all(".boot-dot"),
    ticks: all(".boot-tick"),
    meridians: all(".boot-mer"),
    noseMask: one(".boot-nose-mask"),
    tailMask: one(".boot-tail-mask"),
    cMask: one(".boot-c-mask"),
    edges: all(".boot-edge"),
    glow: one(".boot-spark-glow"),
    spark: one(".boot-spark"),
    trail: all(".boot-trail"),
  };
}

type Nodes = ReturnType<typeof nodesOf>;

function drawRing(n: Nodes, f: BootFrame) {
  const r = CIRCLE_R * f.scale;
  const dotR = DOT_RING_R * f.scale;
  const ring = (radius: number, a: number) => project(onRing(radius, a), f.tilt);
  n.segments.forEach((el, i) => {
    const a = ring(r, ringAngle(i, SEGMENTS));
    const b = ring(r, ringAngle(i + 1, SEGMENTS));
    set(el, { x1: a.x, y1: a.y, x2: b.x, y2: b.y, "stroke-width": 0.6 * a.f, "stroke-opacity": depthAlpha(a.z, r) });
  });
  n.dots.forEach((el, i) => {
    const p = ring(dotR, ringAngle(i, DOTS));
    set(el, { cx: p.x, cy: p.y, r: DOT_SIZE * p.f, "fill-opacity": depthAlpha(p.z, dotR) });
  });
  n.ticks.forEach((el, i) => {
    const a = ring(TICK_IN_R * f.scale, ringAngle(i, 4));
    const b = ring(dotR, ringAngle(i, 4));
    set(el, { x1: a.x, y1: a.y, x2: b.x, y2: b.y, "stroke-opacity": depthAlpha(a.z, r) });
  });
  n.meridians.forEach((el, k) => {
    if (f.meridians < 0.01) return set(el, { "stroke-opacity": 0 });
    const plane = MERIDIAN_PLANES[Math.floor(k / MERIDIAN_SEGMENTS)]!;
    const i = k % MERIDIAN_SEGMENTS;
    const a = project(plane(onRing(r, ringAngle(i, MERIDIAN_SEGMENTS))), f.tilt);
    const b = project(plane(onRing(r, ringAngle(i + 1, MERIDIAN_SEGMENTS))), f.tilt);
    set(el, { x1: a.x, y1: a.y, x2: b.x, y2: b.y, "stroke-opacity": depthAlpha(a.z, r) * 0.6 * f.meridians });
  });
}

function drawCraft(n: Nodes, f: BootFrame) {
  set(n.noseMask, { y: 30 - 19 * f.nose, height: 19 * f.nose });
  set(n.tailMask, { height: 5 * f.tail });
  set(n.cMask, { "stroke-dashoffset": 1 - f.c });
  // A light line on each moving mask's leading edge, as wide as the triangle there.
  const edge = (el: Element, p: number, y: number, half: number) =>
    set(el, p > 0 && p < 1 ? { x1: 32 - half - 0.6, x2: 32 + half + 0.6, y1: y, y2: y, opacity: 0.9 } : { opacity: 0 });
  const [nose, tail] = n.edges;
  if (nose) edge(nose, f.nose, 30 - 19 * f.nose, 3.5 * (1 - f.nose));
  if (tail) edge(tail, f.tail, 41.5 + 5 * f.tail, 2.5 * (1 - f.tail));
}

function drawSpark(n: Nodes, f: BootFrame, now: number, trail: Particle[]) {
  if (f.spark <= 0) {
    trail.length = 0;
    set(n.glow, { opacity: 0 });
    set(n.spark, { opacity: 0 });
    n.trail.forEach((el) => set(el, { opacity: 0 }));
    return;
  }
  const [x, y] = spinePoint(f.c);
  if (f.c < 1) trail.push({ x, y, born: now, vx: (Math.random() - 0.5) * 0.02, vy: (Math.random() - 0.2) * 0.02 });
  while (trail.length > TRAIL || (trail[0] && now - trail[0].born >= TRAIL_MS)) trail.shift();
  n.trail.forEach((el, i) => {
    const s = trail[i];
    if (!s) return set(el, { opacity: 0 });
    const age = now - s.born;
    const life = 1 - age / TRAIL_MS;
    set(el, { cx: s.x + s.vx * age, cy: s.y + s.vy * age, r: 0.35 * life, opacity: life * f.spark });
  });
  const flicker = 0.85 + Math.random() * 0.3;
  set(n.glow, { cx: x, cy: y, r: 3.2 * flicker * f.spark, opacity: 1 });
  set(n.spark, { cx: x, cy: y, r: 0.7 * f.spark, opacity: 1 });
}

/**
 * The app's boot mark assembling itself (specs/2026-10-30-boot-animation-design.md in the internal repo). Its clock
 * runs from `startedAt`, so mounting it again later shows the finished mark with the ring's spin carried on. Every
 * node is made once; frames only set attributes. Still (`animate` off), it shows the finished mark face-on.
 */
export function BootMark({ startedAt, animate }: { startedAt: number; animate: boolean }) {
  const svg = useRef<SVGSVGElement>(null);
  const id = useId().replace(/[^a-zA-Z0-9_-]/g, "");

  useEffect(() => {
    const n = nodesOf(svg.current!);
    const trail: Particle[] = [];
    const draw = (now: number) => {
      const f = bootFrame(now - startedAt);
      drawRing(n, f);
      drawCraft(n, f);
      drawSpark(n, f, now, trail);
    };
    if (!animate) {
      const still = bootFrame(BOOT_MS);
      drawRing(n, { ...still, tilt: { x: 0, y: 0, spin: 0 } });
      drawCraft(n, still);
      drawSpark(n, still, 0, trail);
      return;
    }
    let frame = 0;
    const tick = () => {
      draw(performance.now());
      frame = requestAnimationFrame(tick);
    };
    tick();
    return () => cancelAnimationFrame(frame);
  }, [animate, startedAt]);

  const size = 64 * BOOT_UNIT_PX;
  return (
    <svg
      ref={svg}
      className="boot-mark"
      viewBox="0 0 64 64"
      overflow="visible"
      aria-hidden="true"
      style={{ width: size, height: size, marginTop: PAD_TOP, marginBottom: PAD_BOTTOM }}
    >
      <defs>
        <clipPath id={`${id}-nose`}>
          <rect className="boot-nose-mask" x={0} width={64} y={30} height={0} />
        </clipPath>
        <clipPath id={`${id}-tail`}>
          <rect className="boot-tail-mask" x={0} width={64} y={41.5} height={0} />
        </clipPath>
        <mask id={`${id}-c`} maskUnits="userSpaceOnUse" x={0} y={0} width={64} height={64}>
          <path
            className="boot-c-mask"
            d={C_SPINE}
            pathLength={1}
            fill="none"
            stroke="#fff"
            strokeWidth={4.5}
            strokeDasharray="1 1"
            strokeDashoffset={1}
          />
        </mask>
        <radialGradient id={`${id}-glow`}>
          <stop className="boot-spark-stop" offset={0} />
          <stop offset={0.35} stopColor="currentColor" stopOpacity={0.8} />
          <stop offset={1} stopColor="currentColor" stopOpacity={0} />
        </radialGradient>
      </defs>
      <g className="boot-ring" fill="currentColor" stroke="currentColor">
        {Array.from({ length: SEGMENTS }, (_, i) => (
          <line key={i} className="boot-seg" />
        ))}
        {Array.from({ length: DOTS }, (_, i) => (
          <circle key={i} className="boot-dot" stroke="none" />
        ))}
        {Array.from({ length: 4 }, (_, i) => (
          <line key={i} className="boot-tick" strokeWidth={0.6} />
        ))}
        {MERIDIAN_PLANES.flatMap((_, m) =>
          Array.from({ length: MERIDIAN_SEGMENTS }, (_, i) => (
            <line key={`${m}-${i}`} className="boot-mer" strokeWidth={0.35} strokeOpacity={0} />
          )),
        )}
      </g>
      <g fill="currentColor" stroke="none">
        <polygon points={NOSE} clipPath={`url(#${id}-nose)`} />
        <polygon points={TAIL} clipPath={`url(#${id}-tail)`} />
        <path d={C_PATH} mask={`url(#${id}-c)`} />
      </g>
      <line className="boot-edge" strokeWidth={0.35} opacity={0} />
      <line className="boot-edge" strokeWidth={0.35} opacity={0} />
      {Array.from({ length: TRAIL }, (_, i) => (
        <circle key={i} className="boot-trail" opacity={0} />
      ))}
      <circle className="boot-spark-glow" fill={`url(#${id}-glow)`} opacity={0} />
      <circle className="boot-spark" opacity={0} />
    </svg>
  );
}
