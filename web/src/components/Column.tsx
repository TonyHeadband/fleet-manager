import type { Agent } from "../api/generated/Agent";
import type { ColumnDef } from "../api/generated/ColumnDef";
import type { UiSchema } from "../api/generated/UiSchema";
import { Card, type RunAction } from "./Card";

interface Props {
  def: ColumnDef;
  schema: UiSchema;
  /** The agents that belong in this column; the caller decides which. */
  agents: Agent[];
  now: number;
  onRun: RunAction;
  onOpen: (agent: Agent) => void;
}

/** One column of the board: title, count, and its cards, longest-waiting first. */
export function Column({ def, schema, agents, now, onRun, onOpen }: Props) {
  const cards = [...agents].sort((a, b) => a.status_since - b.status_since);
  return (
    <section className="column" aria-label={def.title} data-tone={def.tone} data-dim={def.dim || undefined}>
      <h2>
        {def.title} <span className="count">{cards.length}</span>
      </h2>
      {cards.length === 0 ? (
        <p className="empty">{def.empty_hint}</p>
      ) : (
        cards.map((a) => <Card key={a.id} schema={schema} agent={a} now={now} onRun={onRun} onOpen={onOpen} />)
      )}
    </section>
  );
}
