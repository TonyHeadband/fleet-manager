import { useState } from "react";
import type { FleetClient } from "../api/client";
import type { FormDef } from "../api/generated/FormDef";
import type { Release } from "../api/generated/Release";
import type { Task } from "../api/generated/Task";
import type { UiSchema } from "../api/generated/UiSchema";
import { inputs, usableForm, type FormValues } from "../api/forms";
import type { Notice } from "../shell/messages";
import { OutcomeLine } from "../tasks/TasksView";
import { taskOutcome, type Live } from "../tasks/model";
import { Button } from "../ui/Button";
import { FilterMenu } from "../ui/FilterMenu";
import { Form } from "../ui/Form";
import { formatDay, localDay, orderedReleases, releaseProgress } from "./model";
import { ReleaseBars } from "./ReleaseBars";

// A port of crates/fleet-gui/src/views/releases.rs and of the release half of app/products.rs.

interface Props {
  client: FleetClient;
  /** The active product; none says "Pick a product". */
  product: string | null;
  releases: Release[];
  tasks: Task[];
  live: Record<string, Live>;
  schema: UiSchema | null;
  /** Unix seconds. */
  now: number;
  say: (notice: Notice, text: string) => void;
  /** A release was saved, changed or deleted: the products are fetched again. */
  onChanged: () => void;
  /** A task moved: the tasks are fetched again. */
  onTaskChanged: () => void;
}

/** The form on screen: a new release, or the release being edited. */
type Editing = { kind: "new" } | { kind: "edit"; release: Release };

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

export function ReleasesView({ client, product, releases, tasks, live, schema, now, say, onChanged, onTaskChanged }: Props) {
  const [editing, setEditing] = useState<Editing | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<number | null>(null);
  const create = schema && usableForm(schema, "release_create");
  const edit = schema && usableForm(schema, "release_edit");
  if (!create || !edit) return null;
  if (product === null)
    return (
      <div className="releases">
        <p className="low">Pick a product</p>
      </div>
    );
  const day = localDay(now);

  const change = (call: () => Promise<unknown>) =>
    void call()
      .catch((e: unknown) => say("error", `Could not update the release: ${message(e)}`))
      .finally(onChanged);
  const move = (t: Task, to: number | null) =>
    void client
      .setTaskRelease(t.id, to)
      .catch((e: unknown) => say("error", `Could not update ${t.name}: ${message(e)}`))
      .finally(onTaskChanged);

  const form = editing && (editing.kind === "new" ? create : edit);
  return (
    <div className="releases">
      {editing && form ? (
        <div className="release-form">
          <Form
            key={editing.kind === "new" ? "new" : editing.release.id}
            form={form}
            today={formatDay(day)}
            initial={editing.kind === "edit" ? valuesFrom(form, editing.release) : undefined}
            onSubmit={async (submit, values) => {
              const params: Record<string, string> = editing.kind === "new" ? { product } : { id: String(editing.release.id) };
              await client.submitForm(form, submit, params, values);
              setEditing(null);
              onChanged();
            }}
            onCancel={() => setEditing(null)}
          />
        </div>
      ) : (
        <div>
          <Button variant="primary" onClick={() => setEditing({ kind: "new" })}>
            New release
          </Button>
        </div>
      )}
      {releases.length === 0 && <p className="low small">No releases</p>}
      {orderedReleases(releases).map((r) => (
        <article key={r.id} className="release-card" aria-label={r.version}>
          <div className="release-title">
            <h2>{r.version}</h2>
            <span className="low small">{`${r.start} to ${r.target}`}</span>
          </div>
          {/* Only this release's tasks count, whatever product the list holds. */}
          <ReleaseBars release={r} progress={releaseProgress(r, tasks, day)} />
          <div className="release-actions">
            {r.released_at === null ? (
              <Button onClick={() => change(() => client.updateRelease(r.id, { released: true }))}>Mark released</Button>
            ) : (
              <>
                <span className="low small">{`Released ${formatDay(localDay(r.released_at))}`}</span>
                <Button className="ghost" onClick={() => change(() => client.updateRelease(r.id, { released: false }))}>
                  Unrelease
                </Button>
              </>
            )}
            <Button className="ghost" onClick={() => setEditing({ kind: "edit", release: r })}>
              Edit
            </Button>
            {confirmDelete === r.id ? (
              <>
                <span className="danger small">{`Delete ${r.version}? Its tasks go to the backlog`}</span>
                <Button
                  variant="danger"
                  onClick={() => {
                    setConfirmDelete(null);
                    change(() => client.deleteRelease(r.id));
                  }}
                >
                  Confirm
                </Button>
                <Button className="ghost" onClick={() => setConfirmDelete(null)}>
                  Cancel
                </Button>
              </>
            ) : (
              <Button variant="quiet-danger" onClick={() => setConfirmDelete(r.id)}>
                Delete
              </Button>
            )}
          </div>
          {tasks
            .filter((t) => t.release === r.id && t.product === product)
            .map((t) => (
              <TaskRow
                key={t.id}
                task={t}
                others={releases.filter((o) => o.id !== r.id && o.released_at === null)}
                live={live}
                onMove={(to) => move(t, to)}
              />
            ))}
        </article>
      ))}
    </div>
  );
}

/** The form's values for a release: each input's key read from the release, as egui's `values_from`. */
function valuesFrom(form: FormDef, r: Release): FormValues {
  const record = r as unknown as Record<string, unknown>;
  return Object.fromEntries(inputs(form).map((i) => [i.key, record[i.key] == null ? "" : String(record[i.key])]));
}

function TaskRow({
  task: t,
  others,
  live,
  onMove,
}: {
  task: Task;
  others: Release[];
  live: Record<string, Live>;
  onMove: (to: number | null) => void;
}) {
  const outcome = taskOutcome(t);
  const locked = outcome === "merged" || outcome === "done_by_hand";
  return (
    <div className="release-task">
      <span className="release-task-name">{t.name}</span>
      <OutcomeLine task={t} stateText="" live={live} />
      {/* A pick in the open menu moves the task; keys on the closed control do nothing, as egui's combo. */}
      <FilterMenu
        summary="Move to…"
        name="Move to…"
        width="release"
        actions
        disabled={locked}
        title={locked ? "A finished task stays in its release" : undefined}
      >
        {(close) =>
          [...others.map((o): [number | null, string] => [o.id, o.version]), [null, "Backlog"] as [number | null, string]].map(
            ([to, text]) => (
              <button
                key={text}
                type="button"
                role="menuitem"
                className="menu-item"
                onClick={() => {
                  close();
                  onMove(to);
                }}
              >
                {text}
              </button>
            ),
          )
        }
      </FilterMenu>
    </div>
  );
}
