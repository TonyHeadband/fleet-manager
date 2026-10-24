import { columnFor } from "../api/schema";
import type { Agent } from "../api/generated/Agent";
import type { UiSchema } from "../api/generated/UiSchema";
import type { RunAction } from "./Card";
import { Column } from "./Column";

interface Props {
  schema: UiSchema;
  agents: Agent[];
  now: number;
  onRun: RunAction;
  onOpen: (agent: Agent) => void;
}

/** Renders whatever columns the daemon's schema declares; no column or status name lives here. */
export function Board({ schema, agents, now, onRun, onOpen }: Props) {
  return (
    <div className="board">
      {schema.columns.map((def) => (
        <Column key={def.id} def={def} schema={schema} agents={agents.filter((a) => columnFor(schema, a)?.id === def.id)} now={now} onRun={onRun} onOpen={onOpen} />
      ))}
    </div>
  );
}
