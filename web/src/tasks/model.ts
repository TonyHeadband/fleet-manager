import type { Agent } from "../api/generated/Agent";
import type { Orphan } from "../api/generated/Orphan";
import type { Status } from "../api/generated/Status";
import type { StopCause } from "../api/generated/StopCause";
import type { Task } from "../api/generated/Task";
import type { TaskList } from "../api/generated/TaskList";
import type { Tone } from "../api/generated/Tone";
import type { UiSchema } from "../api/generated/UiSchema";
import { stopAction } from "../api/schema";
import { allows, type ReleaseFilter } from "../board/filters";
import { formatBytes } from "../shell/counts";
import type { TaskFilter } from "../shell/routes";

// A port of crates/fleet-gui/src/views/tasks.rs's model, with fleet-api's `task_outcome` and `task_progress`.

/** A running task's agent as the board shows it: its column's title and tone, and whether it can be stopped. */
export interface Live {
  status: string;
  tone: Tone;
  canStop: boolean;
}

/** egui's `live_tasks`: every agent on the board by id. */
export function liveTasks(schema: UiSchema | null, agents: Agent[]): Record<string, Live> {
  if (!schema) return {};
  return Object.fromEntries(
    agents.map((a) => {
      const column = schema.columns.find((c) => c.statuses.includes(a.status));
      return [a.id, { status: column?.title ?? debugName(a.status), tone: column?.tone ?? "neutral", canStop: !!stopAction(schema, a) }];
    }),
  );
}

/** Rust's `{:?}` of a status, which egui shows where the board has no column for it. */
const debugName = (s: Status) => s.replace(/(^|_)(\w)/g, (_, _sep: string, c: string) => c.toUpperCase());

export type Outcome = "open" | "merged" | "done_by_hand" | "dropped";

export function taskOutcome(t: Task): Outcome {
  if (t.done_by_hand) return "done_by_hand";
  const prs = t.prs ?? [];
  const open = prs.some((p) => p.state === "open");
  const changed = t.repos.filter((w) => w.changed);
  const merged = changed.length > 0 && changed.every((w) => prs.some((p) => p.repo === w.repo && p.state === "merged"));
  if (merged && !open) return "merged";
  return t.state === "archived" ? "dropped" : "open";
}

/** How an outcome reads on a badge (`outcome_look`); no tone is the neutral badge. */
export const OUTCOME_LOOK: Record<Outcome, [string, Tone | undefined]> = {
  open: ["Open", undefined],
  merged: ["Merged", "done"],
  done_by_hand: ["Done by hand", "done"],
  dropped: ["Dropped", "caution"],
};

/** `3/7 steps` for an orchestrated task, from its last approved breakdown; for a solo one its board column. */
export function progressText(t: Task, live: Record<string, Live>): string {
  if ((t.kind ?? "solo") === "solo") return live[t.id]?.status ?? (t.state === "running" ? "Working" : "Stopped");
  const plan = [...(t.breakdowns ?? [])].reverse().find((b) => b.approval === "approved");
  const done = plan?.steps.filter((s) => s.state === "done").length ?? 0;
  return `${done}/${plan?.steps.length ?? 0} steps`;
}

export type Entry = { kind: "task"; task: Task } | { kind: "orphan"; orphan: Orphan };

export const entryId = (e: Entry) => (e.kind === "task" ? e.task.id : e.orphan.id);

/** A session's name, or the folder's last component. */
export function orphanName(o: Orphan): string {
  return o.path.split("/").filter(Boolean).pop() ?? o.path;
}

export const entryName = (e: Entry) => (e.kind === "task" ? e.task.name : orphanName(e.orphan));

export function entrySize(e: Entry): string {
  const bytes = e.kind === "task" ? (e.task.footprint?.total ?? null) : e.orphan.bytes;
  return bytes === null ? "—" : formatBytes(bytes);
}

/** Running, stopped with crashed first, orphans, archived. */
function group(e: Entry): number {
  if (e.kind === "orphan") return 3;
  const t = e.task;
  if (t.state === "running") return 0;
  if (t.state === "archived") return 4;
  return t.stop_cause === "crashed" ? 1 : 2;
}

const CAUSE_LOOK: Record<StopCause, [string, Tone]> = {
  exited: ["Exited", "active"],
  crashed: ["Crashed", "danger"],
  finished: ["Finished", "active"],
  interrupted: ["Interrupted", "caution"],
  stopped: ["Stopped", "caution"],
};

/** The row's badge (`look`): a running task in its board column's colour, a stopped one in its cause's. */
export function look(e: Entry, live: Record<string, Live>): [string, Tone | undefined] {
  if (e.kind === "orphan") return ["Orphan", "caution"];
  const t = e.task;
  if (t.state === "running") {
    const l = live[t.id];
    return l ? [l.status, l.tone] : ["Running", undefined];
  }
  if (t.state === "archived") return ["Archived", undefined];
  return CAUSE_LOOK[t.stop_cause ?? "stopped"];
}

/** The active product and the board's release filter; with no product nothing narrows the list. */
export interface Scope {
  product: string | null;
  release: ReleaseFilter;
}

const inScope = (s: Scope, t: Task) => s.product === null || t.product === s.product;

/** The rows to show, in list order: newest first within each group, orphans by path (`entries`). */
export function entries(list: TaskList, filter: TaskFilter, archived: boolean, scope: Scope): Entry[] {
  const keep = (t: Task) => {
    switch (filter) {
      case "all":
        return t.state !== "archived" || archived;
      case "running":
        return t.state === "running";
      case "stopped":
        return t.state === "stopped";
      case "crashed":
        return t.state === "stopped" && t.stop_cause === "crashed";
      // An archived task is where a merged or dropped one ends up, so these look past Show archived.
      case "merged":
        return taskOutcome(t) === "merged";
      case "dropped":
        return taskOutcome(t) === "dropped";
      case "orphans":
        return false;
    }
  };
  const rows: Entry[] = list.tasks
    .filter((t) => inScope(scope, t) && allows(scope.release, t.release))
    .filter(keep)
    .map((task) => ({ kind: "task", task }));
  if (filter === "all" || filter === "orphans") rows.push(...list.orphans.map((orphan): Entry => ({ kind: "orphan", orphan })));
  return rows.sort((a, b) => {
    const by = group(a) - group(b);
    if (by !== 0) return by;
    // An unknown start sorts as oldest.
    if (a.kind === "task" && b.kind === "task") return (b.task.created_at ?? -Infinity) - (a.task.created_at ?? -Infinity);
    if (a.kind === "orphan" && b.kind === "orphan") return a.orphan.path < b.orphan.path ? -1 : a.orphan.path > b.orphan.path ? 1 : 0;
    return 0;
  });
}

/** The totals row's values (`totals`): the product's tasks, and every orphan, which belongs to none. */
export interface Totals {
  counts: string[];
  disk: string[];
  clones: string;
  agents: number;
}

export function totals(list: TaskList, scope: Scope): Totals {
  const tasks = list.tasks.filter((t) => inScope(scope, t));
  const size = (t: Task) => t.footprint?.total ?? 0;
  const sum = (ns: number[]) => ns.reduce((a, b) => a + b, 0);
  const running = tasks.filter((t) => t.state === "running");
  const stopped = tasks.filter((t) => t.state === "stopped");
  const crashed = stopped.filter((t) => t.stop_cause === "crashed").length;
  const orphans = list.orphans.length;
  const counts = [`${running.length} running`, `${stopped.length} stopped`];
  if (crashed > 0) counts.push(`${crashed} crashed`);
  if (orphans > 0) counts.push(`${orphans} ${orphans === 1 ? "orphan" : "orphans"}`);
  const orphanBytes = sum(list.orphans.map((o) => o.bytes ?? 0));
  const held = sum(tasks.filter((t) => t.state !== "archived").map(size)) + orphanBytes;
  const reclaimable = sum(stopped.map(size)) + orphanBytes;
  return {
    counts,
    disk: [`${formatBytes(held)} held`, `${formatBytes(reclaimable)} reclaimable`],
    clones: list.clones_bytes === null ? "measuring…" : formatBytes(list.clones_bytes),
    // Records made on upgrade say nothing about when they started, so they are not "since this version".
    agents: sum(tasks.filter((t) => t.created_at !== null).map((t) => t.agents_used)),
  };
}
