// Lite's `fui::radar_mark` (crates/fleet-gui/src/widgets/fui.rs) as an SVG in the same 64-unit square, held still:
// Lite turns it only while animations are on, a setting the web does not have yet.
const HUB = { x: 32, y: 34 };

/** A point `r` units from the hub in direction `a` (0 = up, clockwise). */
function polar(r: number, a: number): [number, number] {
  const t = a - Math.PI / 2;
  return [HUB.x + r * Math.cos(t), HUB.y + r * Math.sin(t)];
}

const pts = (ps: [number, number][]) => ps.map(([x, y]) => `${x.toFixed(2)},${y.toFixed(2)}`).join(" ");

const WEDGE = (35 * Math.PI) / 180;
const fan = pts([polar(0, 0), ...Array.from({ length: 13 }, (_, i) => polar(23, (WEDGE * i) / 12))]);
const DOTS = Math.floor((2 * Math.PI * 26.5) / 3);
const ring = Array.from({ length: DOTS }, (_, i) => polar(26.5, (2 * Math.PI * i) / DOTS));
const ticks = [0, 1, 2, 3].map((i) => [polar(24.5, (Math.PI / 2) * i), polar(26.5, (Math.PI / 2) * i)] as const);

export function RadarMark({ className }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 64 64" aria-hidden="true" fill="currentColor" stroke="currentColor" strokeWidth={0.6}>
      <polygon points={fan} stroke="none" fillOpacity={0.35} />
      <line x1={HUB.x} y1={HUB.y} x2={polar(23, 0)[0]} y2={polar(23, 0)[1]} />
      <circle cx={HUB.x} cy={HUB.y} r={23} fill="none" />
      {ring.map(([x, y], i) => (
        <circle key={i} cx={x} cy={y} r={0.35} stroke="none" />
      ))}
      {ticks.map(([[x1, y1], [x2, y2]], i) => (
        <line key={i} x1={x1} y1={y1} x2={x2} y2={y2} />
      ))}
      <g stroke="none">
        <polygon points="32,11 35.5,30 28.5,30" />
        <polygon points="29.5,41.5 34.5,41.5 32,46.5" />
        <path d="M17 51 A15 18 0 0 1 47 51 A15 15.5 0 0 0 17 51 Z" />
      </g>
    </svg>
  );
}
