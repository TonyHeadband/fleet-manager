import type { Status } from "../../api/generated/Status";
import { ColumnBars, Stat } from "../../charts";
import { TONE } from "../ConversationSelector";
import type { WidgetDef, WidgetProps } from "./widget";
import { Widget } from "./Widget";

const BARS: { status: Status; label: string }[] = [
  { status: "working", label: "Working" },
  { status: "needs_you", label: "Needs you" },
  { status: "review", label: "Review" },
  { status: "talking", label: "Talking" },
  { status: "queued", label: "Queued" },
];

function AgentsByStatus({ persona, agents, loaded }: WidgetProps) {
  // Supervisors are not fleet work, and a stopped agent is no longer an agent to count.
  const live = agents.filter((a) => a.persona === persona && a.kind !== "supervisor" && a.status !== "stopped");
  const bars = BARS.map(({ status, label }) => ({
    label,
    value: live.filter((a) => a.status === status).length,
    paint: { tone: TONE[status] },
  }));
  return (
    <Widget def={AGENTS_BY_STATUS}>
      <div className="widget-stat">
        <Stat label="Agents" value={loaded ? live.length : null} />
      </div>
      <div className="widget-chart">
        <ColumnBars label="Agents by status" bars={bars} />
      </div>
    </Widget>
  );
}

export const AGENTS_BY_STATUS: WidgetDef = {
  id: "agents_by_status",
  title: "Agents by status",
  minW: 2,
  minH: 2,
  render: (p) => <AgentsByStatus {...p} />,
};
