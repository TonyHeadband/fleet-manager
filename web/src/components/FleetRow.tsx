import type { Agent } from "../api/generated/Agent";
import type { UiSchema } from "../api/generated/UiSchema";
import { columnFor, formatDuration, title } from "../api/schema";
import { specialistIcon, statusWord } from "../board/groups";
import { Mark } from "../ui/icons";
import { EdgeMotion } from "./EdgeMotion";

interface Props {
  schema: UiSchema;
  agent: Agent;
  /** What the agent is called: its specialist, by its persona's nickname (`fleetName`). */
  name: string;
  now: number;
  selected: boolean;
  stuck: boolean;
  /** The task has an open pull request. */
  pr: boolean;
  onOpen: (agent: Agent) => void;
}

/**
 * A fleet agent in one row, as Lite's `fleet_row` (the Compact display): its specialist, title and time on its own
 * column's status edge, in a dashed outline. A click anywhere on it opens the agent.
 */
export function FleetRow({ schema, agent, name, now, selected, stuck, pr, onOpen }: Props) {
  const tone = columnFor(schema, agent)?.tone ?? "neutral";
  return (
    <div
      className="fleet-row"
      role="button"
      tabIndex={0}
      aria-label={`${name} ${title(agent)}, ${statusWord(agent.status)}`}
      data-tone={tone}
      data-selected={selected || undefined}
      data-outline={stuck ? "stuck" : selected ? "selected" : ""}
      onClick={() => onOpen(agent)}
      onKeyDown={(e) => {
        if (e.key !== "Enter" && e.key !== " ") return;
        e.preventDefault();
        onOpen(agent);
      }}
    >
      <EdgeMotion agent={agent} tone={tone} />
      <Mark name={specialistIcon(agent)} label={name} />
      <span className="fleet-row-title" title={agent.goal}>
        {title(agent)}
      </span>
      {stuck && <Mark name="warn" label="Needs you" />}
      {pr && <Mark name="pullRequest" label="PR open" />}
      <span className="fleet-row-time">{formatDuration(now - agent.status_since)}</span>
    </div>
  );
}
