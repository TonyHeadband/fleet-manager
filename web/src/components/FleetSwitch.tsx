import type { FleetDisplay } from "../board/groups";
import { ChoiceMenu } from "../ui/ChoiceMenu";

const DISPLAYS: { id: FleetDisplay; name: string }[] = [
  { id: "show", name: "Show" },
  { id: "compact", name: "Compact" },
  { id: "hide", name: "Hide" },
];

/** Lite's `fleet_switch`, as the header's other filters: "Subtasks" (an orchestrator's specialists) and the current display, a menu of the three. */
export function FleetSwitch({ current, onPick }: { current: FleetDisplay; onPick: (d: FleetDisplay) => void }) {
  return <ChoiceMenu label="Subtasks" name="Subtasks display" width="menu" items={DISPLAYS} value={current} onPick={onPick} />;
}
