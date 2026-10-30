import { card, fleet } from "../generated/layout";

export type IconName = "stop" | "pause" | "resume" | "info" | "orchestrator" | "code" | "eye" | "flask" | "specialist" | "pullRequest";

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
      case "code":
        return [-1, 1].map((side) => (
          <polyline
            key={side}
            points={`${c + side * 0.35 * half},${c - 0.75 * half} ${c + side * half},${c} ${c + side * 0.35 * half},${c + 0.75 * half}`}
            fill="none"
            stroke="currentColor"
            strokeWidth={1.3}
          />
        ));
      case "eye":
        return (
          <>
            <ellipse cx={c} cy={c} rx={1.1 * half} ry={0.6 * half} fill="none" stroke="currentColor" strokeWidth={1.2} />
            <circle cx={c} cy={c} r={0.35 * half} />
          </>
        );
      case "flask": {
        const pts = [[-0.3, -1], [0.3, -1], [0.3, -0.3], [1, 1], [-1, 1], [-0.3, -0.3]] as const;
        return <polygon points={pts.map(([x, y]) => `${c + x * half},${c + y * half}`).join(" ")} fill="none" stroke="currentColor" strokeWidth={1.2} />;
      }
      case "specialist":
        return <polygon points={`${c},${c - half} ${c + half},${c} ${c},${c + half} ${c - half},${c}`} fill="none" stroke="currentColor" strokeWidth={1.2} />;
      case "pullRequest":
        return (
          <g fill="none" stroke="currentColor" strokeWidth={1.3}>
            <line x1={c} y1={c + half} x2={c} y2={c - half} />
            <polyline points={`${c - 0.6 * half},${c - 0.4 * half} ${c},${c - half} ${c + 0.6 * half},${c - 0.4 * half}`} />
          </g>
        );
    }
  })();
  return (
    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} fill="currentColor" aria-hidden="true">
      {shape}
    </svg>
  );
}

export type MarkName = Exclude<IconName, "stop" | "pause" | "resume" | "info"> | "warn";

/**
 * Lite's `mark`: a small glyph that names what it marks on hover and to assistive tech. "warn" is Lite's `warn_mark`,
 * an amber triangle with a "!" cut out of it.
 */
export function Mark({ name, label }: { name: MarkName; label: string }) {
  const size = fleet.mark;
  const c = size / 2;
  return (
    <span className="mark" data-mark={name} role="img" aria-label={label} title={label}>
      {name === "warn" ? (
        <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} aria-hidden="true">
          <polygon points={`${c},${c - 6} ${c + 6.5},${c + 5.5} ${c - 6.5},${c + 5.5}`} fill="var(--tone-attention)" />
          <line x1={c} y1={c - 2} x2={c} y2={c + 1.5} stroke="var(--surface)" strokeWidth={1.5} />
          <circle cx={c} cy={c + 3.5} r={0.9} fill="var(--surface)" />
        </svg>
      ) : (
        <Glyph name={name} size={size} />
      )}
    </span>
  );
}
