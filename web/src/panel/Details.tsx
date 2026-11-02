import type { Agent } from "../api/generated/Agent";
import type { UiSchema } from "../api/generated/UiSchema";
import { columnFor } from "../api/schema";
import { badgeItems, shownFields } from "../board/card";
import type { Stage } from "../board/stage";
import { Badge } from "../ui/Badge";
import { Field } from "../ui/Field";
import type { Task } from "../api/generated/Task";
import { useMerge, type MergeApi } from "./MergeButton";
import { navigate, tasksHref } from "../shell/routes";

/**
 * What the card cannot show, under the panel's title: one status row (the column, every badge, the task's stage as
 * on its card, a PR's a link to it, and the size, the time at its right end), then a line per field the schema shows that nothing else says (fields this client does not
 * know), then the model, mode and effort on one line. The header's path names the repos and the branch, the card the persona.
 * A stopped agent's opens in Tasks, where its history is.
 */
export function Details({
  schema,
  agent,
  now,
  stage = null,
  task,
  merge,
  onTaskChanged = () => {},
}: {
  schema: UiSchema;
  agent: Agent;
  now: number;
  stage?: Stage | null;
  /** The task's record, for the Merge button next to the PR link; `merge` is how it reaches the daemon. */
  task?: Task;
  merge?: MergeApi;
  onTaskChanged?: () => void;
}) {
  const mergeControl = useMerge(merge ? task : undefined, merge ?? NO_MERGE, onTaskChanged);
  const shown = shownFields(schema.card_fields, agent);
  const ofKind = (kind: string) => shown.filter((f) => f.kind === kind);
  const column = columnFor(schema, agent);
  const others = shown.filter((f) => (f.kind === "text" || f.kind === "mono") && !["title", "activity", "branch"].includes(f.key));
  // Only a fleet agent has a resolved model; a solo agent runs on whatever its persona's Claude is set to.
  const model = (agent.kind ?? "solo") !== "solo" ? `Model: ${agent.model ?? "persona default"}` : null;
  const run = [model, agent.mode && `Mode: ${agent.mode}`, agent.effort && `Effort: ${agent.effort}`].filter(Boolean).join(" · ");
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
          // The stage says what "has changes" did, and where the work is.
          .filter((f) => f.key !== "repos" && f.key !== "has_changes")
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
        {stage &&
          (stage.url ? (
            <a className="details-stage" href={stage.url} target="_blank" rel="noreferrer">
              <StageBadge stage={stage} />
            </a>
          ) : (
            <StageBadge stage={stage} />
          ))}
        {mergeControl.button}
        {ofKind("bytes").map((f) => (
          <Field key={f.key} def={f} agent={agent} now={now} />
        ))}
        {ofKind("duration").map((f) => (
          <span key={f.key} className="details-time">
            <Field def={f} agent={agent} now={now} />
          </span>
        ))}
      </div>
      {mergeControl.outcome}
      {others.map((f) => (
        <div key={f.key} className="details-field">
          <span className="mono-label">{f.label}</span>
          <Field def={f} agent={agent} now={now} />
        </div>
      ))}
      {run && <p className="details-meta">{run}</p>}
      {agent.status === "stopped" && (
        <button type="button" className="link" onClick={() => navigate(tasksHref("all", agent.id))}>
          Open in Tasks
        </button>
      )}
    </div>
  );
}

const StageBadge = ({ stage }: { stage: Stage }) => (
  <Badge label={stage.title} field="stage" tone={stage.tone}>
    {stage.text}
  </Badge>
);

const NO_MERGE: MergeApi = { refresh: () => Promise.reject(new Error("no merge")), merge: () => Promise.reject(new Error("no merge")) };
