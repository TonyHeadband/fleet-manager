import type { Agent } from "../api/generated/Agent";
import type { ColumnDef } from "../api/generated/ColumnDef";
import type { Persona } from "../api/generated/Persona";
import type { UiSchema } from "../api/generated/UiSchema";
import { title } from "../api/schema";
import { fleetName, orchestrated, since, taskId, type FleetDisplay, type Group } from "../board/groups";
import { Card, type RunAction } from "./Card";
import { FleetRow } from "./FleetRow";

export interface TaskActions {
  onStopTask?: (task: string, fleet: number) => void;
  onRestart?: (task: string) => void;
}

const nothing = () => {};

interface Props extends TaskActions {
  def: ColumnDef;
  schema: UiSchema;
  /** The tasks that belong in this column; the caller decides which. */
  groups: Group[];
  now: number;
  selected?: string | null;
  tags?: Record<string, string>;
  display?: FleetDisplay;
  personas?: Persona[];
  /** Whether each task, by id, has an open pull request. */
  prs?: Record<string, boolean>;
  /** Whether a lead's fleet shows; `isOpen` and `onFold` are keyed by the lead's id. */
  isOpen?: (g: Group) => boolean;
  onFold?: (leadId: string) => void;
  onRun: RunAction;
  onOpen: (agent: Agent) => void;
}

/**
 * One column of the board, as Lite's `column`: title, count of tasks, and its tasks longest-waiting first. An
 * orchestrated task's fleet follows its card, indented: full cards (Show), one row each (Compact), or none (Hide).
 */
export function Column(props: Props) {
  const { def, schema, now, selected = null, tags = {}, display = "compact", personas = [], prs = {}, isOpen = () => true, onFold = nothing, onStopTask = nothing, onRestart = nothing, onRun, onOpen } = props;
  const cards = [...props.groups].sort((a, b) => since(a) - since(b));
  return (
    <section className="column" aria-label={def.title} data-tone={def.tone} data-dim={def.dim || undefined}>
      <h2 className="column-header">
        <span className="dot" data-tone={def.tone} aria-hidden="true" />
        <span className="mono-label">{def.title}</span>{" "}
        <span className="mono-label count" data-tone={def.tone} aria-label={String(cards.length)}>
          {String(cards.length).padStart(2, "0")}
        </span>
        <span className="ticks" aria-hidden="true" />
      </h2>
      <div className="column-body">
        {cards.length === 0 && <p className="empty">{def.empty_hint}</p>}
        {cards.map((g) => {
          const a = g.lead;
          const common = { schema, now, onRun, onOpen };
          if (!orchestrated(g)) return <Card key={a.id} {...common} agent={a} selected={a.id === selected} tag={tags[a.persona]} />;
          const open = isOpen(g);
          // Until the fleet view is loaded, the lead's activity says the task has a PR open, as in Lite.
          const pr = prs[taskId(a)] ?? a.activity === "PR open";
          return (
            <div key={a.id} className="task">
              <Card
                {...common}
                agent={a}
                selected={a.id === selected}
                tag={tags[a.persona]}
                kind={{ type: "lead", group: g, display, open, onFold: () => onFold(a.id), onStopTask, onRestart }}
              />
              {display !== "hide" && open && g.fleet.length > 0 && (
                <div className="fleet">
                  {g.fleet.map((f) => {
                    const stuck = f.status === "needs_you";
                    const name = fleetName(f, personas);
                    return display === "show" ? (
                      <Card key={f.id} {...common} agent={f} selected={f.id === selected} tag={name} kind={{ type: "fleet", task: title(a), stuck }} />
                    ) : (
                      <FleetRow key={f.id} schema={schema} agent={f} name={name} now={now} selected={f.id === selected} stuck={stuck} pr={pr} onOpen={onOpen} />
                    );
                  })}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </section>
  );
}
