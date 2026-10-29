import type { TaskList } from "../api/generated/TaskList";

/** The strip's task counts. A port of Lite's `Counts::of` (crates/fleet-gui/src/views/kanban.rs). */
export interface Counts {
  running: number;
  /** Every cause, crashed included, as the Tasks view's totals count them. */
  stopped: number;
  crashed: number;
  orphans: number;
  /** What the tasks and orphans still hold on disk; archived ones hold nothing. */
  held: number;
}

export function taskCounts(list: TaskList): Counts {
  const c: Counts = { running: 0, stopped: 0, crashed: 0, orphans: list.orphans.length, held: list.orphans.reduce((n, o) => n + (o.bytes ?? 0), 0) };
  for (const t of list.tasks) {
    if (t.state === "archived") continue;
    if (t.state === "running") c.running += 1;
    else {
      c.stopped += 1;
      if (t.stop_cause === "crashed") c.crashed += 1;
    }
    c.held += t.footprint?.total ?? 0;
  }
  return c;
}

const UNITS = ["KB", "MB", "GB", "TB", "PB"];

/** A port of `fleet_api::task::format_bytes`: SI units, one decimal. */
export function formatBytes(bytes: number): string {
  if (bytes < 1000) return `${bytes} B`;
  let value = bytes / 1000;
  let unit = 0;
  while (value >= 999.95 && unit + 1 < UNITS.length) {
    value /= 1000;
    unit += 1;
  }
  return `${value.toFixed(1)} ${UNITS[unit]}`;
}
