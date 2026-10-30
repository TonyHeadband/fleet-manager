import type { Release } from "../api/generated/Release";
import type { Tone } from "../api/generated/Tone";
import type { ReleaseProgress } from "./model";

// egui's `release_bars` (views/releases.rs), drawn by the Releases cards and Management's Overview.

const plural = (n: number, unit: string) => `${n} ${unit}${n === 1 ? "" : "s"}`;

function Bar({ fraction, tone }: { fraction: number; tone: Tone }) {
  return (
    <span className="release-bar" data-tone={tone} aria-hidden="true">
      <span style={{ width: `${Math.min(Math.max(fraction, 0), 1) * 100}%` }} />
    </span>
  );
}

/** The release's two bars with their captions: tasks merged out of those that count, and days used (or overdue). */
export function ReleaseBars({ release: r, progress: p }: { release: Release; progress: ReleaseProgress }) {
  const done = p.counted > 0 && p.merged === p.counted;
  return (
    <div className="release-bars">
      <span className="release-bar-pair">
        <Bar fraction={p.counted === 0 ? 0 : p.merged / p.counted} tone={done ? "done" : "active"} />
        <span className="small">{`${p.merged}/${p.counted} tasks`}</span>
      </span>
      <span className="release-bar-pair">
        {p.overdueDays > 0 && r.released_at === null ? (
          <>
            <Bar fraction={1} tone="attention" />
            <span className="small" data-tone="attention">{`${plural(p.overdueDays, "day")} overdue`}</span>
          </>
        ) : (
          <>
            <Bar fraction={p.daysUsed / Math.max(p.daysTotal, 1)} tone="active" />
            <span className="small">{`${p.daysUsed}/${p.daysTotal} days`}</span>
          </>
        )}
      </span>
    </div>
  );
}
