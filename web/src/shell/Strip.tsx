import type { TaskList } from "../api/generated/TaskList";
import type { Tone } from "../api/generated/Tone";
import { formatBytes, taskCounts } from "./counts";

/**
 * The row under the board's header: "Tasks: ● N running ● N stopped [N crashed] [N orphan] · <disk>" at its far end,
 * once the tasks list has been fetched, as Lite's `task_counts` draws it. The stopped dot takes the Stopped column's
 * tone. Lite opens Tasks filtered to a clicked count; the web has no Tasks view yet, so the counts are plain text.
 */
export function Strip({ tasks, stoppedTone }: { tasks: TaskList | null; stoppedTone: Tone }) {
  const c = tasks && taskCounts(tasks);
  return (
    <section className="strip" aria-label="Tasks strip">
      {c && (
        <div className="task-counts">
          <span className="low">Tasks:</span>
          <span className="count-item">
            <span className="dot" data-tone="done" aria-hidden="true" />
            <span>{`${c.running} running`}</span>
          </span>
          <span className="count-item">
            <span className="dot" data-tone={stoppedTone} aria-hidden="true" />
            <span>{`${c.stopped} stopped`}</span>
          </span>
          {c.crashed > 0 && (
            <span className="badge" data-tone="danger" title="Show crashed tasks">{`${c.crashed} crashed`}</span>
          )}
          {c.orphans > 0 && (
            <span className="badge" data-tone="caution" title="Show orphans">{`${c.orphans} orphan`}</span>
          )}
          <span className="low">{`· ${formatBytes(c.held)}`}</span>
        </div>
      )}
    </section>
  );
}
