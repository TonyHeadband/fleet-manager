import { useEffect, type ReactNode } from "react";
import type { Persona } from "../api/generated/Persona";
import type { Task } from "../api/generated/Task";
import type { TaskList } from "../api/generated/TaskList";
import type { Tone } from "../api/generated/Tone";
import { personaLabels } from "../board/filters";
import type { Load } from "../shell/choices";
import { navigate, tasksHref, type TaskFilter } from "../shell/routes";
import { Badge } from "../ui/Badge";
import { Button } from "../ui/Button";
import {
  OUTCOME_LOOK,
  entries,
  entryId,
  entryName,
  entrySize,
  look,
  progressText,
  taskOutcome,
  totals,
  type Entry,
  type Live,
  type Scope,
} from "./model";

interface Props {
  load: Load<TaskList>;
  onRetry: () => void;
  live: Record<string, Live>;
  personas: Persona[];
  scope: Scope;
  showArchived: boolean;
  filter: TaskFilter;
  /** The task or orphan id the address picks. */
  task: string | null;
  /** The passport of the selected row; none draws "Nothing to show". */
  passport: (e: Entry, badge: [string, Tone | undefined]) => ReactNode;
}

/** egui's `tasks::main`: with no list yet, Loading… or the error with Retry; with one, totals, list and passport. */
export function TasksView({ load, onRetry, ...rest }: Props) {
  if (load.state === "ready") return <WithList list={load.value} error={load.error ?? null} {...rest} />;
  return (
    <div className="tasks-unloaded">
      {load.state === "failed" ? (
        <>
          <p role="alert">{`Cannot load the tasks: ${load.error ?? "unknown error"}`}</p>
          <Button variant="primary" onClick={onRetry}>
            Retry
          </Button>
        </>
      ) : (
        <p className="loading">Loading…</p>
      )}
    </div>
  );
}

type ListProps = Omit<Props, "load" | "onRetry"> & { list: TaskList; error: string | null };

function WithList({ list, error, live, personas, scope, showArchived, filter, task, passport }: ListProps) {
  const rows = entries(list, filter, showArchived, scope);
  // A selection the list no longer shows (archived and hidden, filtered out, gone) moves to the first row.
  const selected = rows.find((e) => entryId(e) === task) ?? rows[0];
  const selectedId = selected ? entryId(selected) : null;
  useEffect(() => {
    if (selectedId !== task) navigate(tasksHref(filter, selectedId), true);
  }, [selectedId, task, filter]);
  const labels = personaLabels(personas, [...new Set([...personas.map((p) => p.id), ...list.tasks.map((t) => t.persona)])]);
  const t = totals(list, scope);

  return (
    <div className="tasks">
      <section className="tasks-totals" aria-label="Totals">
        <div className="stats">
          <Stat caption="Tasks" values={t.counts} />
          <Stat caption="Disk" values={t.disk} />
          <Stat caption="Clones cache" values={[t.clones]} />
          <Stat caption="Agents started" values={[String(t.agents)]} />
          <Stat caption="Tokens · cost" values={["not tracked yet"]} />
        </div>
        {error !== null && <p className="refresh-error">{`Could not refresh: ${error}`}</p>}
      </section>
      <div className="tasks-main">
        <div className="tasks-list">
          {filter !== "all" && (
            <div className="tasks-filter">
              <span>{`Showing ${filter}`}</span>
              <Button className="ghost" onClick={() => navigate(tasksHref("all", selectedId))}>
                Show all
              </Button>
            </div>
          )}
          {rows.length === 0 && <p className="tasks-note">No tasks</p>}
          <ul aria-label="Tasks">
            {rows.map((e) => {
              const id = entryId(e);
              const badge = look(e, live);
              return (
                <li key={id}>
                  <button
                    type="button"
                    className="task-row"
                    aria-label={entryName(e)}
                    aria-pressed={id === selectedId}
                    data-archived={e.kind === "task" && e.task.state === "archived" ? "true" : undefined}
                    onClick={() => navigate(tasksHref(filter, id))}
                  >
                    <span className="task-row-text">
                      <span className="task-row-name">{entryName(e)}</span>
                      {e.kind === "task" && (
                        <>
                          <OutcomeLine task={e.task} stateText={badge[0]} live={live} />
                          {/* Every task runs under one, whatever the board's persona filter. */}
                          <span className="low">{labels.get(e.task.persona) ?? e.task.persona}</span>
                        </>
                      )}
                    </span>
                    <Badge label="State" tone={badge[1]}>
                      {badge[0]}
                    </Badge>
                    <span className="low">{entrySize(e)}</span>
                  </button>
                </li>
              );
            })}
          </ul>
        </div>
        <section className="passport" aria-label="Passport">
          {selected ? passport(selected, look(selected, live)) : <p className="tasks-note">Nothing to show</p>}
        </section>
      </div>
    </div>
  );
}

/** A caption in egui's mono uppercase over its values. */
function Stat({ caption, values }: { caption: string; values: string[] }) {
  return (
    <div className="stat">
      <span className="mono-label">{caption}</span>
      <span className="stat-values">
        {values.map((v) => (
          <span key={v}>{v}</span>
        ))}
      </span>
    </div>
  );
}

/** The outcome badge and the progress, which a running solo task already shows as its state badge (`outcome_line`). */
export function OutcomeLine({ task, stateText, live }: { task: Task; stateText: string; live: Record<string, Live> }) {
  const [text, tone] = OUTCOME_LOOK[taskOutcome(task)];
  const progress = progressText(task, live);
  return (
    <span className="outcome-line">
      <Badge label="Outcome" tone={tone}>
        {text}
      </Badge>
      {progress !== stateText && <span className="progress">{progress}</span>}
    </span>
  );
}
