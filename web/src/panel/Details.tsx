import type { Agent } from "../api/generated/Agent";
import type { UiSchema } from "../api/generated/UiSchema";
import { columnFor } from "../api/schema";
import { badgeItems, shownFields } from "../board/card";
import { Badge } from "../ui/Badge";
import { Field } from "../ui/Field";
import { workspaceTip } from "./workspaceTip";

/**
 * Lite's `details_block`: what the fixed-size card cannot show. The status and time, every badge and the size, then
 * a line per other field the schema shows (the branch, fields this client does not know), then the model and the
 * mode and effort. The title above, the terminal below and the Repos row show the task, the activity and the repos.
 */
export function Details({ schema, agent, now }: { schema: UiSchema; agent: Agent; now: number }) {
  const shown = shownFields(schema.card_fields, agent);
  const ofKind = (kind: string) => shown.filter((f) => f.kind === kind);
  const column = columnFor(schema, agent);
  const others = shown.filter((f) => (f.kind === "text" || f.kind === "mono") && f.key !== "title" && f.key !== "activity");
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
        {ofKind("duration").map((f) => (
          <Field key={f.key} def={f} agent={agent} now={now} />
        ))}
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
    </div>
  );
}

/** Lite's `repos_row`: a badge per workspace, saying where its branch started. Repos come from the product, so
 * nothing is added here. */
export function ReposRow({ agent }: { agent: Agent }) {
  return (
    <div className="repos-row">
      <span className="mono-label">Repos</span>
      {agent.workspaces.map((w) => (
        <Badge key={w.repo} label={workspaceTip(w)}>
          {w.repo}
        </Badge>
      ))}
    </div>
  );
}
