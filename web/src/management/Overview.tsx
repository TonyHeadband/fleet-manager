import type { Agent } from "../api/generated/Agent";
import type { Product } from "../api/generated/Product";
import type { Status } from "../api/generated/Status";
import type { Task } from "../api/generated/Task";
import type { Tone } from "../api/generated/Tone";
import { rollup, taskId } from "../board/groups";
import { localDay, releaseProgress } from "../releases/model";
import { ReleaseBars } from "../releases/ReleaseBars";
import type { Load } from "../shell/choices";
import { taskOutcome } from "../tasks/model";
import { Button } from "../ui/Button";
import { FilterMenu } from "../ui/FilterMenu";

// A port of crates/fleet-gui/src/views/management/overview.rs: every product at a glance, read-only. The only action
// is picking the active product.

/** A task's status is its fleet's most urgent agent's (`setup.rs`'s `by_task` and `rollup`), by task id. */
export function taskStatuses(agents: Agent[]): Record<string, Status> {
  const byTask: Record<string, Status[]> = {};
  for (const a of agents) (byTask[taskId(a)] ??= []).push(a.status);
  return Object.fromEntries(Object.entries(byTask).map(([id, s]) => [id, rollup(s)]));
}

export interface Counts {
  working: number;
  needsYou: number;
  review: number;
  merged: number;
}

/** egui's `counts`: merged and done-by-hand tasks are merged; any other with a live agent counts by its status. */
export function overviewCounts(product: string, tasks: Task[], statuses: Record<string, Status>): Counts {
  const c: Counts = { working: 0, needsYou: 0, review: 0, merged: 0 };
  for (const t of tasks.filter((t) => t.product === product)) {
    const o = taskOutcome(t);
    if (o === "merged" || o === "done_by_hand") c.merged += 1;
    else if (statuses[t.id] === "working") c.working += 1;
    else if (statuses[t.id] === "needs_you") c.needsYou += 1;
    else if (statuses[t.id] === "review") c.review += 1;
  }
  return c;
}

interface Props {
  products: Load<Product[]>;
  tasks: Task[];
  agents: Agent[];
  active: string | null;
  onActive: (id: string) => void;
  /** The one product shown; null is All. Kept by the app, as egui's OverviewState. */
  shown: string | null;
  onShown: (id: string | null) => void;
  /** Unix seconds. */
  now: number;
}

export function Overview({ products, tasks, agents, active, onActive, shown, onShown, now }: Props) {
  if (products.state !== "ready")
    return (
      <div className="tabbed-page">
        <p className="note">Loading…</p>
      </div>
    );
  const list = products.value;
  // A product that was deleted no longer narrows anything.
  const filter = list.some((p) => p.id === shown) ? shown : null;
  const statuses = taskStatuses(agents);
  const day = localDay(now);
  const rows: [string | null, string, string][] = [
    [null, "All products", "Show all products"],
    ...list.map((p): [string, string, string] => [p.id, p.name, `Show ${p.name}`]),
  ];
  return (
    <div className="tabbed-page overview">
      <div className="overview-filters">
        <FilterMenu label="Product" summary={list.find((p) => p.id === filter)?.name ?? "All products"} name="Product" width="release">
          {(close) =>
            rows.map(([id, name, label]) => (
              <button
                key={label}
                type="button"
                className="menu-item"
                aria-label={label}
                aria-pressed={id === filter}
                onClick={() => {
                  close();
                  onShown(id);
                }}
              >
                {name}
              </button>
            ))
          }
        </FilterMenu>
      </div>
      {list.length === 0 && <p className="note">No products yet</p>}
      {list
        .filter((p) => filter === null || p.id === filter)
        .map((p) => {
          const c = overviewCounts(p.id, tasks, statuses);
          const counts: [string, number, Tone][] = [
            ["Working", c.working, "active"],
            ["Needs you", c.needsYou, "attention"],
            ["Review", c.review, "done"],
            ["Merged", c.merged, "done"],
          ];
          return (
            <article key={p.id} className="page-card overview-card" aria-label={p.name}>
              <div className="overview-head">
                <Button className="ghost" onClick={() => onActive(p.id)}>
                  {p.name}
                </Button>
                {active === p.id && <span className="note">active</span>}
                <span className="overview-counts">
                  {counts.map(([name, n, tone]) => (
                    <span key={name} data-tone={n > 0 ? tone : undefined}>{`${name} ${n}`}</span>
                  ))}
                </span>
              </div>
              {p.releases
                .filter((r) => r.released_at === null)
                .map((r) => (
                  <div key={r.id} className="overview-release">
                    <span className="overview-version">{r.version}</span>
                    <ReleaseBars release={r} progress={releaseProgress(r, tasks, day)} />
                  </div>
                ))}
            </article>
          );
        })}
    </div>
  );
}
