import type { TaskList } from "../api/generated/TaskList";
import type { Tone } from "../api/generated/Tone";
import { formatBytes, taskCounts } from "./counts";
import { tasksHref, type TaskFilter } from "./routes";

/**
 * The row under the board's header: "Tasks: ● N running ● N stopped [N crashed] [N orphan] · <disk>" at its far end,
 * once the tasks list has been fetched, as Lite's `task_counts` draws it; each count opens Tasks on its filter. The
 * stopped dot takes the Stopped column's tone.
 */
export function Strip({ tasks, stoppedTone, selected }: { tasks: TaskList | null; stoppedTone: Tone; selected: string | null }) {
  // As egui's `open_tasks(filter, None)`: the selection stays if the filtered list still shows it.
  const href = (f: TaskFilter) => tasksHref(f, selected);
  const c = tasks && taskCounts(tasks);
  return (
    <section className="strip" aria-label="Tasks strip">
      {c && (
        <div className="task-counts">
          <span className="low">Tasks:</span>
          <a className="count-item" href={href("running")}>
            <span className="dot" data-tone="done" aria-hidden="true" />
            <span>{`${c.running} running`}</span>
          </a>
          <a className="count-item" href={href("stopped")}>
            <span className="dot" data-tone={stoppedTone} aria-hidden="true" />
            <span>{`${c.stopped} stopped`}</span>
          </a>
          {c.crashed > 0 && (
            <a className="badge" data-tone="danger" href={href("crashed")} title="Show crashed tasks">{`${c.crashed} crashed`}</a>
          )}
          {c.orphans > 0 && (
            <a className="badge" data-tone="caution" href={href("orphans")} title="Show orphans">{`${c.orphans} orphan`}</a>
          )}
          <span className="low">{`· ${formatBytes(c.held)}`}</span>
        </div>
      )}
    </section>
  );
}
