import { useEffect, useMemo, useState } from "react";
import type { Agent } from "../api/generated/Agent";
import type { Persona } from "../api/generated/Persona";
import type { UiSchema } from "../api/generated/UiSchema";
import { columnOfGroup, groups as groupsOf, stuck, type FleetDisplay, type Group } from "../board/groups";
import type { RunAction } from "./Card";
import { Column, type TaskActions } from "./Column";

interface Props extends TaskActions {
  schema: UiSchema;
  agents: Agent[];
  now: number;
  /** The open card's agent id. */
  selected?: string | null;
  /** The detail panel stands beside the board: the quiet column gives it its place, unless the open card is in it. */
  rail?: boolean;
  /** `personaTags` of the shown agents. */
  tags?: Record<string, string>;
  display?: FleetDisplay;
  personas?: Persona[];
  prs?: Record<string, boolean>;
  onRun: RunAction;
  onOpen: (agent: Agent) => void;
  /** A click on the board that lands on no card, row or control. */
  onEmpty?: () => void;
}

/** What a board click lands on that is not the board's own empty space. */
const TAKEN = ".card, .fleet-row, .task, .fleet, button, a, input, select, textarea";

/**
 * Renders whatever columns the daemon's schema declares, with the agents grouped into tasks as Lite's board does; no
 * column or status name lives here.
 */
export function Board({ schema, agents, rail = false, onStopTask, onRestart, onEmpty, ...rest }: Props) {
  const groups = useMemo(() => groupsOf(agents), [agents]);
  // Folded tasks by lead id, kept here so a task keeps its fold when it moves column (Lite's `CardState` map).
  const [folded, setFolded] = useState<ReadonlySet<string>>(new Set());
  // As Lite's `task_card`: a fleet agent that needs you unfolds its task, for good.
  const stuckLeads = groups.filter((g) => stuck(g) !== null).map((g) => g.lead.id);
  const stuckKey = stuckLeads.join("\n");
  useEffect(() => {
    const ids = stuckKey.split("\n");
    setFolded((f) => (ids.some((id) => f.has(id)) ? new Set([...f].filter((id) => !ids.includes(id))) : f));
  }, [stuckKey]);
  const isOpen = (g: Group) => !folded.has(g.lead.id) || stuck(g) !== null;
  const onFold = (id: string) =>
    setFolded((f) => {
      const next = new Set(f);
      if (!next.delete(id)) next.add(id);
      return next;
    });
  return (
    <div
      className="board"
      onClick={(e) => {
        if (!(e.target as Element).closest(TAKEN)) onEmpty?.();
      }}
    >
      {schema.columns.map((def) => {
        const mine = groups.filter((g) => columnOfGroup(schema, g)?.id === def.id);
        const holdsOpen = mine.some((g) => [g.lead, ...g.fleet].some((a) => a.id === rest.selected));
        return (
          <Column
            key={def.id}
            def={def}
            schema={schema}
            groups={mine}
            rail={rail && def.dim && !holdsOpen}
            isOpen={isOpen}
            onFold={onFold}
            onStopTask={onStopTask}
            onRestart={onRestart}
            {...rest}
          />
        );
      })}
    </div>
  );
}
