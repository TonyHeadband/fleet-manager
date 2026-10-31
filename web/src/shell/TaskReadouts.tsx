import { useLayoutEffect, useRef, useState, type RefObject } from "react";
import type { TaskList } from "../api/generated/TaskList";
import type { Tone } from "../api/generated/Tone";
import { pad2, Readout, ReadoutRail } from "../ui/Readout";
import { formatBytes, taskCounts } from "./counts";
import { tasksHref, type TaskFilter } from "./routes";

/**
 * Which cells fit `room`: the lowest rank gives way first, a whole cell at a time, until the rest fit. `widths` and
 * `ranks` are by cell, in the rail's order.
 */
export function visibleCells(widths: number[], ranks: number[], room: number): Set<number> {
  const shown = new Set(widths.map((_, i) => i));
  let total = widths.reduce((n, w) => n + w, 0);
  for (const i of [...shown].sort((a, b) => (ranks[a] ?? 0) - (ranks[b] ?? 0))) {
    if (total <= room) break;
    shown.delete(i);
    total -= widths[i] ?? 0;
  }
  return shown;
}

/** Each cell's rank: on a narrow header DISK gives way first, TASKS last. */
const RANK = { tasks: 6, run: 5, stop: 4, crash: 3, orphan: 2, disk: 1 } as const;
type Cell = keyof typeof RANK;

/**
 * The cells the header has room for once its other children (title, filters, switch) have theirs; null until measured,
 * and where there is no ResizeObserver (tests), which shows them all. A hidden cell keeps the width it last had shown.
 */
function useRoom(rail: RefObject<HTMLDivElement | null>, cells: Cell[]): Set<Cell> | null {
  const [shown, setShown] = useState<Set<Cell> | null>(null);
  const widths = useRef(new Map<Cell, number>());
  const key = cells.join(" ");
  useLayoutEffect(() => {
    const el = rail.current;
    const header = el?.parentElement;
    if (!el || !header || typeof ResizeObserver === "undefined") return;
    const measure = () => {
      const nodes = [...el.querySelectorAll<HTMLElement>(".readout")];
      cells.forEach((c, i) => {
        const n = nodes[i];
        if (n && !n.hidden) widths.current.set(c, n.offsetWidth);
      });
      const style = getComputedStyle(header);
      const others = [...header.children].filter((c) => c !== el) as HTMLElement[];
      const gap = parseFloat(style.columnGap) || 0;
      const room =
        header.clientWidth -
        (parseFloat(style.paddingLeft) || 0) -
        (parseFloat(style.paddingRight) || 0) -
        others.reduce((n, c) => n + c.scrollWidth + gap, 0);
      const fit = visibleCells(
        cells.map((c) => widths.current.get(c) ?? 0),
        cells.map((c) => RANK[c]),
        room,
      );
      const next = new Set(cells.filter((_, i) => fit.has(i)));
      setShown((s) => (s && s.size === next.size && [...next].every((c) => s.has(c)) ? s : next));
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(header);
    return () => ro.disconnect();
    // `key` stands for `cells`, a new array each render.
  }, [rail, key]);
  return shown;
}

/**
 * The board's task counts, a readout rail at the header's right end, once the tasks list has been fetched, as Lite's
 * `task_counts` counts them; each count opens Tasks on its filter. The stopped bar takes the Stopped column's tone.
 */
export function TaskReadouts({ tasks, stoppedTone, selected }: { tasks: TaskList | null; stoppedTone: Tone; selected: string | null }) {
  // As egui's `open_tasks(filter, None)`: the selection stays if the filtered list still shows it.
  const href = (f: TaskFilter) => tasksHref(f, selected);
  const c = tasks && taskCounts(tasks);
  const cells: Cell[] = c
    ? ["tasks", "run", "stop", ...(c.crashed > 0 ? ["crash" as const] : []), ...(c.orphans > 0 ? ["orphan" as const] : []), "disk"]
    : [];
  const rail = useRef<HTMLDivElement>(null);
  const shown = useRoom(rail, cells);
  const hidden = (cell: Cell) => shown !== null && !shown.has(cell);
  const count = (n: number) => ({ label: String(n), children: pad2(n) });
  return (
    c && (
      <ReadoutRail label="Tasks" ref={rail}>
        <Readout k="Tasks" hidden={hidden("tasks")} {...count(c.running + c.stopped)} />
        <Readout k="Run" tone="done" href={href("running")} hidden={hidden("run")} {...count(c.running)} />
        <Readout k="Stop" tone={stoppedTone} href={href("stopped")} hidden={hidden("stop")} {...count(c.stopped)} />
        {c.crashed > 0 && (
          <Readout
            k="Crash"
            tone="danger"
            href={href("crashed")}
            title="Show crashed tasks"
            hidden={hidden("crash")}
            {...count(c.crashed)}
          />
        )}
        {c.orphans > 0 && (
          <Readout k="Orphan" tone="caution" href={href("orphans")} title="Show orphans" hidden={hidden("orphan")} {...count(c.orphans)} />
        )}
        <Readout k="Disk" hidden={hidden("disk")}>
          {formatBytes(c.held)}
        </Readout>
      </ReadoutRail>
    )
  );
}
