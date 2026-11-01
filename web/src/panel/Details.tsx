import type { Agent } from "../api/generated/Agent";
import type { UiSchema } from "../api/generated/UiSchema";
import { columnFor } from "../api/schema";
import { badgeItems, shownFields } from "../board/card";
import { Badge } from "../ui/Badge";
import { Field } from "../ui/Field";
import { navigate, tasksHref } from "../shell/routes";

/**
 * What the card cannot show, under the panel's title: one status row (the column, every badge and the size, the time
 * at its right end), then a line per field the schema shows that nothing else says (fields this client does not
 * know), the model and the mode and effort. The header's path names the repos and the branch, the card the persona.
 * A stopped agent's opens in Tasks, where its history is.
 */
export function Details({ schema, agent, now }: { schema: UiSchema; agent: Agent; now: number }) {
  const shown = shownFields(schema.card_fields, agent);
  const ofKind = (kind: string) => shown.filter((f) => f.kind === kind);
  const column = columnFor(schema, agent);
  const others = shown.filter((f) => (f.kind === "text" || f.kind === "mono") && !["title", "activity", "branch"].includes(f.key));
  const modeEffort = [agent.mode && `Mode: ${agent.mode}`, agent.effort && `Effort: ${agent.effort}`].filter(Boolean).join(" · ");
  return (
    <div className="details">
      <div className="details-status">
        {column && (
          <span className="details-column">
            <span className="dot" data-tone={column.tone} aria-hidden="true" />
            <span className="mono-label">{column.title}</span>
          </span>
        )}
        {ofKind("badge")
          .filter((f) => f.key !== "repos")
          .flatMap((f) =>
            badgeItems(f, agent)
              // A stopped task's cause reads "Stopped" under the column's own "Stopped".
              .filter((item) => item.text !== column?.title)
              .map((item) => (
                <Badge key={`${f.key}:${item.text}`} label={f.label} field={f.key} tone={item.tone ?? undefined}>
                  {item.text}
                </Badge>
              )),
          )}
        {ofKind("bytes").map((f) => (
          <Field key={f.key} def={f} agent={agent} now={now} />
        ))}
        {ofKind("duration").map((f) => (
          <span key={f.key} className="details-time">
            <Field def={f} agent={agent} now={now} />
          </span>
        ))}
      </div>
      {others.map((f) => (
        <div key={f.key} className="details-field">
          <span className="mono-label">{f.label}</span>
          <Field def={f} agent={agent} now={now} />
        </div>
      ))}
      {/* Only a fleet agent has a resolved model; a solo agent runs on whatever its persona's Claude is set to. */}
      {(agent.kind ?? "solo") !== "solo" && <p className="details-meta">{`Model: ${agent.model ?? "persona default"}`}</p>}
      {modeEffort && <p className="details-meta">{modeEffort}</p>}
      {agent.status === "stopped" && (
        <button type="button" className="link" onClick={() => navigate(tasksHref("all", agent.id))}>
          Open in Tasks
        </button>
      )}
    </div>
  );
}
