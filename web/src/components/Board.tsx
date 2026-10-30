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
  /** `personaTags` of the shown agents. */
  tags?: Record<string, string>;
  display?: FleetDisplay;
  personas?: Persona[];
  prs?: Record<string, boolean>;
  onRun: RunAction;
  onOpen: (agent: Agent) => void;
}

/**
 * Renders whatever columns the daemon's schema declares, with the agents grouped into tasks as Lite's board does; no
 * column or status name lives here.
 */
export function Board({ schema, agents, onStopTask, onRestart, ...rest }: Props) {
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
    <div className="board">
      {schema.columns.map((def) => (
        <Column
          key={def.id}
          def={def}
          schema={schema}
          groups={groups.filter((g) => columnOfGroup(schema, g)?.id === def.id)}
          isOpen={isOpen}
          onFold={onFold}
          onStopTask={onStopTask}
          onRestart={onRestart}
          {...rest}
        />
      ))}
    </div>
  );
}
