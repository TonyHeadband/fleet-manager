import { columnFor, fieldValue, title, visibleActions } from "../api/schema";
import type { ActionDef } from "../api/generated/ActionDef";
import type { Agent } from "../api/generated/Agent";
import type { UiSchema } from "../api/generated/UiSchema";
import { iconOf, repoSummary, shownFields, slotOf, stateBadge, useConfirm, type Slot } from "../board/card";
import { Badge } from "../ui/Badge";
import { Button } from "../ui/Button";
import { Field } from "../ui/Field";
import { SmallIconButton } from "../ui/SmallIconButton";

export type RunAction = (action: ActionDef, agent: Agent, input?: string) => void;

interface Props {
  schema: UiSchema;
  agent: Agent;
  now: number;
  /** The card whose details the side panel shows. */
  selected?: boolean;
  /** The agent's persona, shown only when the board holds more than one (`personaTags`). */
  tag?: string;
  onRun: RunAction;
  onOpen: (agent: Agent) => void;
}

/**
 * One agent's card, as Lite's `card`: four fixed rows (title and time; state badge, changes, repo and size;
 * activity; persona and actions), so every card is the same height whatever its content.
 */
export function Card({ schema, agent, now, selected = false, tag, onRun, onOpen }: Props) {
  const confirm = useConfirm();
  const tone = columnFor(schema, agent)?.tone ?? "neutral";
  const shown = shownFields(schema.card_fields, agent);
  const inSlot = (slot: Slot) => shown.filter((f) => slotOf(f) === slot);
  const badge = stateBadge(schema.card_fields, agent);
  const repos = inSlot("repos").map((f) => repoSummary(fieldValue(agent, f.key) as string[]))[0];
  // Input actions (Answer) live in the side panel's reply box, not on the card.
  const actions = visibleActions(schema, agent).filter((a) => a.input === null);

  const control = (action: ActionDef) => {
    const armed = action.destructive && confirm.armed(action.id);
    const press = () => confirm.press(action.id, action.destructive) && onRun(action, agent, undefined);
    const icon = iconOf(action);
    const label = armed ? (icon ? `Click again to ${action.label.toLowerCase()}` : `Confirm ${action.label}`) : action.label;
    return (
      <span key={action.id} className="card-action" data-confirming={armed || undefined}>
        {icon ? (
          <SmallIconButton icon={icon} label={label} danger={armed} onClick={press} />
        ) : (
          <Button
            variant={armed ? "danger" : action.destructive ? "quiet-danger" : "default"}
            onClick={(e) => {
              e.stopPropagation();
              press();
            }}
          >
            {label}
          </Button>
        )}
      </span>
    );
  };

  return (
    <article className="card" data-tone={tone} data-selected={selected || undefined} onClick={() => onOpen(agent)}>
      <div className="card-row" data-row="title">
        <span className="card-left card-title" title={agent.goal}>
          {title(agent)}
        </span>
        {inSlot("duration").map((f) => (
          <Field key={f.key} def={f} agent={agent} now={now} />
        ))}
      </div>
      <div className="card-row" data-row="state">
        <span className="card-left">
          {badge && (
            <Badge label={badge.field.label} field={badge.field.key} tone={badge.tone ?? undefined}>
              {badge.text}
            </Badge>
          )}
          {inSlot("changes").map((f) => (
            <span key={f.key} className="changes-dot" role="img" aria-label={f.label} title={f.label} />
          ))}
          {repos && (
            <>
              <span className="repo" title={repos.all}>
                {repos.first}
              </span>
              {repos.more && (
                <span className="repo-more" title={repos.all}>
                  {repos.more}
                </span>
              )}
            </>
          )}
        </span>
        {inSlot("size").map((f) => (
          <Field key={f.key} def={f} agent={agent} now={now} />
        ))}
      </div>
      <div className="card-row" data-row="activity">
        {inSlot("activity").map((f) => (
          <span key={f.key} className="card-left card-activity">
            {String(fieldValue(agent, f.key))}
          </span>
        ))}
      </div>
      <div className="card-row" data-row="actions">
        <span className="mono-label card-tag" title={tag}>
          {tag}
        </span>
        <span className="card-actions">
          {agent.status === "stopped" && <SmallIconButton icon="info" label="Details" title="not on the web yet" disabled onClick={() => {}} />}
          {actions.map(control)}
        </span>
      </div>
      {selected && (
        <span className="brackets" aria-hidden="true">
          {["nw", "ne", "sw", "se"].map((c) => (
            <span key={c} className="bracket" data-corner={c} />
          ))}
        </span>
      )}
    </article>
  );
}
