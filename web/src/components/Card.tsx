import { useState } from "react";
import { columnFor, evalCondition, visibleActions } from "../api/schema";
import type { ActionDef } from "../api/generated/ActionDef";
import type { Agent } from "../api/generated/Agent";
import type { UiSchema } from "../api/generated/UiSchema";
import { Button } from "../ui/Button";
import { Field } from "../ui/Field";

export type RunAction = (action: ActionDef, agent: Agent, input?: string) => void;

interface Props {
  schema: UiSchema;
  agent: Agent;
  now: number;
  onRun: RunAction;
  onOpen: (agent: Agent) => void;
}

export function Card({ schema, agent, now, onRun, onOpen }: Props) {
  const [typed, setTyped] = useState<Record<string, string>>({});
  const [confirming, setConfirming] = useState<string | null>(null);
  const tone = columnFor(schema, agent)?.tone ?? "neutral";
  const fields = schema.card_fields.filter((f) => evalCondition(f.show_when, agent));

  const control = (action: ActionDef) => {
    if (action.input !== null) {
      const text = typed[action.id] ?? "";
      return (
        <form
          key={action.id}
          className="action-input"
          onSubmit={(e) => {
            e.preventDefault();
            if (!text.trim()) return;
            onRun(action, agent, text);
            setTyped((t) => ({ ...t, [action.id]: "" }));
          }}
        >
          <input aria-label={action.label} placeholder={action.label} value={text} onChange={(e) => setTyped((t) => ({ ...t, [action.id]: e.target.value }))} />
          <Button type="submit" disabled={!text.trim()}>
            {action.label}
          </Button>
        </form>
      );
    }
    if (action.destructive) {
      return confirming === action.id ? (
        <Button key={action.id} variant="danger" onClick={() => { setConfirming(null); onRun(action, agent, undefined); }}>
          {`Confirm ${action.label}`}
        </Button>
      ) : (
        <Button key={action.id} variant="quiet-danger" onClick={() => setConfirming(action.id)}>
          {action.label}
        </Button>
      );
    }
    return (
      <Button key={action.id} onClick={() => onRun(action, agent, undefined)}>
        {action.label}
      </Button>
    );
  };

  return (
    <article className="card" data-tone={tone}>
      <div className="card-main" role="button" tabIndex={0} onClick={() => onOpen(agent)} onKeyDown={(e) => e.key === "Enter" && onOpen(agent)}>
        {fields.map((f) => (
          <Field key={f.key} def={f} agent={agent} now={now} />
        ))}
      </div>
      <div className="card-actions">{visibleActions(schema, agent).map(control)}</div>
    </article>
  );
}
