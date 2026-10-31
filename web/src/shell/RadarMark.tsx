import type { CSSProperties } from "react";
import {
  C_PATH,
  CIRCLE_R,
  DOT_RING_R,
  DOT_SIZE,
  DOTS,
  HUB,
  NOSE,
  onRing,
  RING_SPILL_BOTTOM,
  RING_TURN_MS,
  ringAngles,
  TAIL,
  TICK_IN_R,
} from "./mark";

// The finished mark: the boot animation's end state (startup/BootMark draws the assembly). Its ring turns only where
// asked (`spin`): the drawer's watermark and the browser's intro, while animations are on.

const at = (r: number, a: number): [number, number] => {
  const [x, y] = onRing(r, a);
  return [HUB.x + x, HUB.y + y];
};

const dots = ringAngles(DOTS).map((a) => at(DOT_RING_R, a));
const ticks = ringAngles(4).map((a) => [at(TICK_IN_R, a), at(DOT_RING_R, a)] as const);

export function RadarMark({ className, spin = false }: { className?: string; spin?: boolean }) {
  return (
    <svg
      className={className}
      viewBox="0 0 64 64"
      overflow="visible"
      // The ring spills past the square; a placement that must keep it whole lifts the mark by this much.
      style={{ "--mark-spill-bottom": RING_SPILL_BOTTOM } as CSSProperties}
      aria-hidden="true"
      fill="currentColor"
      stroke="currentColor"
      strokeWidth={0.6}
    >
      {/* The ring turns about its hub; the craft stays still. */}
      <g
        className="radar-ring"
        data-spin={spin || undefined}
        style={{ transformOrigin: `${HUB.x}px ${HUB.y}px`, animationDuration: spin ? `${RING_TURN_MS}ms` : undefined }}
      >
        <circle cx={HUB.x} cy={HUB.y} r={CIRCLE_R} fill="none" />
        {dots.map(([x, y], i) => (
          <circle key={i} cx={x} cy={y} r={DOT_SIZE} stroke="none" />
        ))}
        {ticks.map(([[x1, y1], [x2, y2]], i) => (
          <line key={i} x1={x1} y1={y1} x2={x2} y2={y2} />
        ))}
      </g>
      <g stroke="none">
        <polygon points={NOSE} />
        <polygon points={TAIL} />
        <path d={C_PATH} />
      </g>
    </svg>
  );
}
