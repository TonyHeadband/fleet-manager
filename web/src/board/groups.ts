import type { Agent } from "../api/generated/Agent";
import type { ColumnDef } from "../api/generated/ColumnDef";
import type { Persona } from "../api/generated/Persona";
import type { Status } from "../api/generated/Status";
import type { UiSchema } from "../api/generated/UiSchema";

// Lite's task grouping (crates/fleet-client/src/schema.rs `Group`, `groups`; crates/fleet-api/src/fleet.rs), ported so
// both boards place a task and its fleet the same way.

/** How the board draws an orchestrated task's fleet: full cards, one-row entries, or only the task card's tally. */
export type FleetDisplay = "show" | "compact" | "hide";

/** The mark a fleet agent is drawn with. */
export type SpecialistMark = "orchestrator" | "code" | "eye" | "flask" | "specialist";

/** An agent's task: its own id unless it belongs to another's. */
export function taskId(a: Agent): string {
  return a.task ?? a.id;
}

const URGENCY: Record<Status, number> = { needs_you: 4, working: 3, talking: 3, queued: 2, review: 1, stopped: 0 };

/** The most urgent of `statuses`: what a task with these agents shows as. Talking reads as Working; none, Stopped. */
export function rollup(statuses: Status[]): Status {
  let top: Status = "stopped";
  for (const s of statuses) if (URGENCY[s] > URGENCY[top]) top = s;
  return top === "talking" ? "working" : top;
}

/** A task on the board: its lead and the rest of its agents, stuck ones first. `status` is the column it sits in. */
export interface Group {
  lead: Agent;
  fleet: Agent[];
  status: Status;
}

const kindOf = (a: Agent) => a.kind ?? "solo";

export function liveFleet(g: Group): Agent[] {
  return g.fleet.filter((a) => a.status !== "stopped");
}

/** The orchestrator stopped while its fleet still runs, or ended on its own before the task was done. */
export function leadDown(g: Group): boolean {
  const { lead } = g;
  return (
    lead.status === "stopped" &&
    (liveFleet(g).length > 0 || (kindOf(lead) === "orchestrator" && (lead.stop_cause === "crashed" || lead.stop_cause === "exited")))
  );
}

export function canRestart(g: Group): boolean {
  return kindOf(g.lead) === "orchestrator" && g.lead.status === "stopped";
}

/** The fleet agent that sends the task to Needs you, when the lead itself does not. */
export function stuck(g: Group): Agent | null {
  if (g.lead.status === "needs_you") return null;
  return g.fleet.find((a) => a.status === "needs_you") ?? null;
}

export function orchestrated(g: Group): boolean {
  return kindOf(g.lead) === "orchestrator" || g.fleet.length > 0;
}

/** Since when the task has been in its column: the longest any of its live agents has been in that status. */
export function since(g: Group): number {
  const inColumn = [g.lead, ...liveFleet(g)].filter((a) => rollup([a.status]) === g.status).map((a) => a.status_since);
  return inColumn.length > 0 ? Math.min(...inColumn) : g.lead.status_since;
}

/**
 * The board's tasks, in order of first appearance. A task's lead is its live orchestrator (the newest when several),
 * else its newest orchestrator, else its first agent; the rest are its fleet.
 */
export function groups(agents: Agent[]): Group[] {
  const byTask = new Map<string, Agent[]>();
  for (const a of agents) {
    const members = byTask.get(taskId(a));
    if (members) members.push(a);
    else byTask.set(taskId(a), [a]);
  }
  return [...byTask.values()].map((members) => {
    let leadAt = 0;
    let best: [boolean, number] | null = null;
    members.forEach((a, i) => {
      if (kindOf(a) !== "orchestrator") return;
      const key: [boolean, number] = [a.status !== "stopped", i];
      if (!best || key[0] > best[0] || (key[0] === best[0] && key[1] > best[1])) {
        best = key;
        leadAt = i;
      }
    });
    const lead = members[leadAt]!;
    const fleet = members
      .filter((_, i) => i !== leadAt)
      .sort((a, b) => Number(a.status !== "needs_you") - Number(b.status !== "needs_you") || a.status_since - b.status_since);
    const g: Group = { lead, fleet, status: "stopped" };
    g.status = leadDown(g) ? "needs_you" : rollup([lead.status, ...liveFleet(g).map((a) => a.status)]);
    return g;
  });
}

/** The column a task sits in: the first whose statuses hold its rolled-up status. */
export function columnOfGroup(schema: UiSchema, g: Group): ColumnDef | undefined {
  return schema.columns.find((c) => c.statuses.includes(g.status));
}

export function specialistIcon(a: Agent): SpecialistMark {
  if (kindOf(a) === "orchestrator") return "orchestrator";
  switch (a.role) {
    case "rust":
    case "typescript-react":
      return "code";
    case "reviewer":
      return "eye";
    case "test-writer":
      return "flask";
    default:
      return "specialist";
  }
}

/** "Ferris (rust)" where the persona gave the specialist a nickname, else the specialist's name. */
export function specialistLabel(nick: string | undefined, specialist: string): string {
  return nick ? `${nick} (${specialist})` : specialist;
}

/** What a fleet agent is called: its specialist, by the nickname its persona gave it. */
export function fleetName(a: Agent, personas: Persona[]): string {
  const specialist = a.role ?? "specialist";
  return specialistLabel(personas.find((p) => p.id === a.persona)?.nicknames?.[specialist], specialist);
}

/** A fleet row's status in words, for assistive tech. */
export function statusWord(s: Status): string {
  return { needs_you: "needs you", working: "working", talking: "working", queued: "queued", review: "review", stopped: "stopped" }[s];
}
