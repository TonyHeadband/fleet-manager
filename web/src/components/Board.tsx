import { columnFor } from "../api/schema";
import type { Agent } from "../api/generated/Agent";
import type { UiSchema } from "../api/generated/UiSchema";
import type { RunAction } from "./Card";
import { Column } from "./Column";

interface Props {
  schema: UiSchema;
  agents: Agent[];
  now: number;
  /** The open card's agent id. */
  selected?: string | null;
  /** `personaTags` of the shown agents. */
  tags?: Record<string, string>;
  onRun: RunAction;
  onOpen: (agent: Agent) => void;
}

/** Renders whatever columns the daemon's schema declares; no column or status name lives here. */
export function Board({ schema, agents, now, selected, tags, onRun, onOpen }: Props) {
  return (
    <div className="board">
      {schema.columns.map((def) => (
        <Column key={def.id} def={def} schema={schema} agents={agents.filter((a) => columnFor(schema, a)?.id === def.id)} now={now} selected={selected} tags={tags} onRun={onRun} onOpen={onOpen} />
      ))}
    </div>
  );
}
