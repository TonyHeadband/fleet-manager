import { card } from "../generated/layout";

export type IconName = "stop" | "pause" | "resume" | "info" | "orchestrator";

/**
 * Lite's painted glyphs (crates/fleet-gui/src/widgets/icon.rs `glyph`) as SVG, centred in a `size` square and
 * reaching `half` from the centre each way, in `currentColor`.
 */
export function Glyph({ name, size = card.iconButton, half = card.iconGlyph }: { name: IconName; size?: number; half?: number }) {
  const c = size / 2;
  const shape = (() => {
    switch (name) {
      case "stop": {
        const s = half * 1.6;
        return <rect x={c - s / 2} y={c - s / 2} width={s} height={s} rx={1} />;
      }
      case "pause": {
        const [w, h] = [half * 0.55, half * 1.8];
        return [-half * 0.5, half * 0.5].map((x) => <rect key={x} x={c + x - w / 2} y={c - h / 2} width={w} height={h} rx={0.5} />);
      }
      case "resume": {
        // Nudged right: a triangle's visual centre sits left of its box's.
        const x = c + half * 0.15;
        return <polygon points={`${x - half * 0.75},${c - half} ${x + half},${c} ${x - half * 0.75},${c + half}`} />;
      }
      case "info":
        return (
          <>
            <circle cx={c} cy={c} r={half + 1} fill="none" stroke="currentColor" strokeWidth={1.2} />
            <circle cx={c} cy={c - half * 0.5} r={0.9} />
            <line x1={c} y1={c - half * 0.1} x2={c} y2={c + half * 0.6} stroke="currentColor" strokeWidth={1.3} />
          </>
        );
      case "orchestrator":
        return (
          <>
            <circle cx={c} cy={c} r={half + 0.5} fill="none" stroke="currentColor" strokeWidth={1.2} />
            <circle cx={c} cy={c} r={half * 0.45} />
          </>
        );
    }
  })();
  return (
    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} fill="currentColor" aria-hidden="true">
      {shape}
    </svg>
  );
}
