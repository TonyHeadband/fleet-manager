import type { Paint } from "../../charts";
import { SegmentedBar } from "../../charts";
import { formatBytes } from "../../shell/counts";
import type { WidgetDef, WidgetProps } from "./widget";
import { Widget } from "./Widget";

/** Footprint kinds in a fixed order, so a kind keeps its colour whatever else is on disk. */
const KINDS = ["worktrees", "build output", "internal", "daemon files", "other files"];

/** A worktree is named for its repo ("core worktree"); they are one kind here. */
const kindOf = (label: string) => (label.endsWith(" worktree") ? "worktrees" : label);

function StorageWidget({ persona, tasks }: WidgetProps) {
  const sums = new Map<string, number>();
  for (const t of tasks) {
    if (t.persona !== persona || t.kind === "supervisor" || t.archived_at !== null || t.state === "archived") continue;
    for (const i of t.footprint?.items ?? []) sums.set(kindOf(i.label), (sums.get(kindOf(i.label)) ?? 0) + i.bytes);
  }
  const known = KINDS.filter((k) => sums.has(k));
  const unknown = [...sums.keys()].filter((k) => !KINDS.includes(k)).sort();
  const segments = [...known, ...unknown].map((label) => {
    const slot = KINDS.indexOf(label);
    const paint: Paint = { series: slot >= 0 && slot < 5 ? (slot as 0 | 1 | 2 | 3 | 4) : "other" };
    return { label, value: sums.get(label) ?? 0, paint };
  });
  return (
    <Widget def={STORAGE}>
      <div className="widget-chart">
        {/* No disk size is known, so the scale runs to the total. */}
        <SegmentedBar label="Storage" segments={segments} max={0} format={formatBytes} />
      </div>
    </Widget>
  );
}

export const STORAGE: WidgetDef = {
  id: "storage",
  title: "Storage",
  source: "task footprint",
  minW: 3,
  minH: 1,
  render: (p) => <StorageWidget {...p} />,
};
