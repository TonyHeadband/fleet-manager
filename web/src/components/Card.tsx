import { columnFor, fieldValue, title, visibleActions } from "../api/schema";
import type { ActionDef } from "../api/generated/ActionDef";
import type { Agent } from "../api/generated/Agent";
import type { UiSchema } from "../api/generated/UiSchema";
import { iconOf, repoSummary, shownFields, slotOf, stateBadge, useConfirm, type Slot } from "../board/card";
import { canRestart, columnOfGroup, leadDown, liveFleet, rollup, specialistIcon, stuck, taskId, type FleetDisplay, type Group } from "../board/groups";
import type { Status } from "../api/generated/Status";
import { card as cardSize, fleet as fleetSize } from "../generated/layout";
import { Mark } from "../ui/icons";
import { Badge } from "../ui/Badge";
import { Button } from "../ui/Button";
import { Field } from "../ui/Field";
import { SmallIconButton } from "../ui/SmallIconButton";
import { navigate, tasksHref } from "../shell/routes";

export type RunAction = (action: ActionDef, agent: Agent, input?: string) => void;

/** What a card is besides an agent's own card, as Lite's `Kind`: a task's lead, or a fleet agent drawn in full. */
export type CardKind =
  | { type: "solo" }
  | {
      type: "lead";
      group: Group;
      display: FleetDisplay;
      /** The fleet shows under the card. */
      open: boolean;
      onFold: () => void;
      onStopTask: (task: string, fleet: number) => void;
      onRestart: (task: string) => void;
    }
  | { type: "fleet"; task: string; stuck: boolean };

interface Props {
  schema: UiSchema;
  agent: Agent;
  now: number;
  /** The card whose details the side panel shows. */
  selected?: boolean;
  /** The agent's persona, shown only when the board holds more than one (`personaTags`). */
  tag?: string;
  kind?: CardKind;
  onRun: RunAction;
  onOpen: (agent: Agent) => void;
}

/** The lead's badge when its task's column comes from somewhere else than the lead's own status (`lead_badge`). */
function leadBadge(schema: UiSchema, g: Group): { stuck: Agent | null; text: string } | null {
  if (leadDown(g)) return { stuck: null, text: "Orchestrator stopped · Restart?" };
  const s = stuck(g);
  if (!s) return null;
  const reason = stateBadge(schema.card_fields.filter((f) => f.key === "blocked_reason"), s)?.text ?? "Needs you";
  return { stuck: s, text: `${title(s)} · ${reason}` };
}

/** The tally's order: the roll-up's, most urgent first. */
const TALLY: Status[] = ["needs_you", "working", "queued", "review", "stopped"];

/** One dot and count per status present in the fleet (`tally`). */
function Tally({ schema, fleet }: { schema: UiSchema; fleet: Agent[] }) {
  return (
    <span className="card-left tally">
      {TALLY.map((status) => {
        const n = fleet.filter((a) => rollup([a.status]) === status).length;
        const col = schema.columns.find((c) => c.statuses.includes(status));
        if (!col || n === 0) return null;
        return (
          <span key={status} className="tally-entry">
            <span className="dot tally-dot" data-tone={col.tone} role="img" aria-label={`${n} ${col.title}`} title={col.title} />
            {n}
          </span>
        );
      })}
    </span>
  );
}

/** `▾ N fleet` (open) or `▸ N fleet`, Lite's `fold_toggle`; the triangle is drawn as Lite paints it. */
function FoldToggle({ n, open, onFold }: { n: number; open: boolean; onFold: () => void }) {
  const [w, h] = [fleetSize.foldText, cardSize.rowTally];
  const [cx, cy] = [4, h / 2];
  const points = open ? [[-3.5, -2], [3.5, -2], [0, 2.5]] : [[-2, -3.5], [2.5, 0], [-2, 3.5]];
  return (
    <button
      type="button"
      className="fold"
      aria-label={`${open ? "▾" : "▸"} ${n} fleet`}
      onClick={(e) => {
        e.stopPropagation();
        onFold();
      }}
    >
      <svg width={w} height={h} viewBox={`0 0 ${w} ${h}`} aria-hidden="true">
        <polygon points={points.map(([x, y]) => `${cx + x!},${cy + y!}`).join(" ")} fill="currentColor" />
      </svg>
      {n} fleet
    </button>
  );
}

/**
 * One agent's card, as Lite's `card`: four fixed rows (title and time; state badge, changes, repo and size;
 * activity; persona and actions), so every card is the same height whatever its content. A task's lead adds the
 * fleet's tally row and the task's own controls; a fleet agent drawn in full adds its `↳ task` line and a dashed
 * outline.
 */
export function Card({ schema, agent, now, selected = false, tag, kind = { type: "solo" }, onRun, onOpen }: Props) {
  const confirm = useConfirm();
  const lead = kind.type === "lead" ? kind : null;
  // A task card is coloured by the column its group sits in, so a stuck fleet agent reads as Needs you.
  const tone = (lead ? columnOfGroup(schema, lead.group) : columnFor(schema, agent))?.tone ?? "neutral";
  const shown = shownFields(schema.card_fields, agent);
  const inSlot = (slot: Slot) => shown.filter((f) => slotOf(f) === slot);
  const badge = stateBadge(schema.card_fields, agent);
  const taskBadge = lead && leadBadge(schema, lead.group);
  const repos = inSlot("repos").map((f) => repoSummary(fieldValue(agent, f.key) as string[]))[0];
  // Input actions (Answer) live in the side panel's reply box, not on the card. A task card's Stop is the whole
  // task's, drawn after them.
  const actions = visibleActions(schema, agent).filter((a) => a.input === null && (!lead || a.id !== "stop"));

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
    <article
      className="card"
      data-kind={kind.type}
      data-tone={tone}
      data-selected={selected || undefined}
      // A full fleet card selects like any card (the accent frame); its dashes show while unselected, in the attention
      // tone while it needs you.
      data-outline={kind.type === "fleet" && !selected ? (kind.stuck ? "stuck" : "") : undefined}
      role={lead ? "group" : undefined}
      aria-label={lead ? `${title(agent)}, ${lead.group.fleet.length} fleet agents` : undefined}
      tabIndex={0}
      onClick={() => onOpen(agent)}
      onKeyDown={(e) => {
        // Only the card's own keys: an action button inside it keeps Enter and Space for itself.
        if (e.target !== e.currentTarget || (e.key !== "Enter" && e.key !== " ")) return;
        e.preventDefault();
        onOpen(agent);
      }}
    >
      <div className="card-in">
      {kind.type === "fleet" && (
        <div className="card-row" data-row="parent">
          <span className="card-left card-parent" title={`↳ ${kind.task}`}>
            ↳ {kind.task}
          </span>
        </div>
      )}
      <div className="card-row" data-row="title">
        <span className="card-left">
          {lead && <Mark name="orchestrator" label="Orchestrator" />}
          {kind.type === "fleet" && <Mark name={specialistIcon(agent)} label="Specialist" />}
          <span className="card-title" title={agent.goal}>
            {title(agent)}
          </span>
        </span>
        {inSlot("duration").map((f) => (
          <Field key={f.key} def={f} agent={agent} now={now} />
        ))}
      </div>
      <div className="card-row" data-row="state">
        <span className="card-left">
          {taskBadge ? (
            <>
              {taskBadge.stuck && <Mark name={specialistIcon(taskBadge.stuck)} label="Specialist" />}
              <Badge label="Task" tone="attention">
                {taskBadge.text}
              </Badge>
            </>
          ) : (
            badge && (
              <Badge label={badge.field.label} field={badge.field.key} tone={badge.tone ?? undefined}>
              {badge.text}
              </Badge>
            )
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
      {lead && (
        <div className="card-row" data-row="tally">
          <Tally schema={schema} fleet={lead.group.fleet} />
          {lead.display !== "hide" && lead.group.fleet.length > 0 && <FoldToggle n={lead.group.fleet.length} open={lead.open} onFold={lead.onFold} />}
        </div>
      )}
      <div className="card-row" data-row="actions">
        <span className="mono-label card-tag" title={tag}>
          {tag}
        </span>
        <span className="card-actions">
          {agent.status === "stopped" && <SmallIconButton icon="info" label="Details" onClick={() => navigate(tasksHref("all", agent.id))} />}
          {actions.map(control)}
          {lead && <TaskControls group={lead.group} onStopTask={lead.onStopTask} onRestart={lead.onRestart} />}
        </span>
      </div>
      </div>
      {/* The right-hand corners only: the status edge owns the left side. */}
      {selected && (
        <span className="brackets" aria-hidden="true">
          {["ne", "se"].map((c) => (
            <span key={c} className="bracket" data-corner={c} />
          ))}
        </span>
      )}
    </article>
  );
}

/** A task card's own actions (`task_controls`): Restart orchestrator once it has ended, then Stop, which stops the
 * whole task after a dialog, while any of its agents runs. */
function TaskControls({ group, onStopTask, onRestart }: { group: Group; onStopTask: (task: string, fleet: number) => void; onRestart: (task: string) => void }) {
  const task = taskId(group.lead);
  const fleet = liveFleet(group).length;
  return (
    <>
      {canRestart(group) && <SmallIconButton icon="resume" label="Restart orchestrator" onClick={() => onRestart(task)} />}
      {(group.lead.status !== "stopped" || fleet > 0) && <SmallIconButton icon="stop" label="Stop" onClick={() => onStopTask(task, fleet)} />}
    </>
  );
}
