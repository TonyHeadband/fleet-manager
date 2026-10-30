import type { FleetDisplay } from "../board/groups";
import { Chip } from "../ui/Chip";

const DISPLAYS: [FleetDisplay, string][] = [
  ["show", "Show"],
  ["compact", "Compact"],
  ["hide", "Hide"],
];

/** "Fleet" and one chip per display, the current one selected (Lite's `fleet_switch`). */
export function FleetSwitch({ current, onPick }: { current: FleetDisplay; onPick: (d: FleetDisplay) => void }) {
  return (
    <div className="fleet-switch">
      <span className="mono-label">Fleet</span>
      <span className="chips">
        {DISPLAYS.map(([d, label]) => (
          <Chip key={d} selected={d === current} onClick={() => d !== current && onPick(d)}>
            {label}
          </Chip>
        ))}
      </span>
    </div>
  );
}
