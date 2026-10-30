import type { Dispatch, SetStateAction } from "react";
import type { FleetClient } from "../api/client";
import type { FormDef } from "../api/generated/FormDef";
import type { Release } from "../api/generated/Release";
import type { Task } from "../api/generated/Task";
import type { UiSchema } from "../api/generated/UiSchema";
import { initialValues, inputs, usableForm, type FormValues } from "../api/forms";
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
  state: ReleasesState;
  onState: Dispatch<SetStateAction<ReleasesState>>;
  say: (notice: Notice, text: string) => void;
  /** A release was saved, changed or deleted: the products are fetched again. */
  onChanged: () => void;
  /** A task moved: the tasks are fetched again. */
  onTaskChanged: () => void;
}

/** The form on screen: a new release, or the release being edited. */
type Editing = { kind: "new" } | { kind: "edit"; release: Release };

/** egui's `ReleaseForm`: what was typed, and which open it is, so each open starts the form afresh. */
type ReleaseForm = Editing & { values: FormValues; opened: number };

/** egui's `ReleasesState`, kept above the view: leaving it keeps an open form, with what was typed, and a pending
 * Delete. */
export interface ReleasesState {
  form: ReleaseForm | null;
  /** The release whose Delete waits for Confirm. */
  confirmDelete: number | null;
  /** The releases with a Mark released, Unrelease or Delete out: their buttons wait for it, so a double click sends
   * once. */
  calling: number[];
}

export const NO_RELEASES_STATE: ReleasesState = { form: null, confirmDelete: null, calling: [] };

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

export function ReleasesView({
  client,
  product,
  releases,
  tasks,
  live,
  schema,
  now,
  state: st,
  onState,
  say,
  onChanged,
  onTaskChanged,
}: Props) {
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

  const set = (patch: Partial<ReleasesState>) => onState((s) => ({ ...s, ...patch }));
  // Edit fills the form from the release every time, as egui's, even over the release already being edited.
  const open = (editing: Editing) =>
    onState((s) => ({
      ...s,
      form: {
        ...editing,
        values: editing.kind === "edit" ? valuesFrom(edit, editing.release) : initialValues(create, formatDay(day)),
        opened: (s.form?.opened ?? 0) + 1,
      },
    }));
  const change = (id: number, call: () => Promise<unknown>) => {
    onState((s) => ({ ...s, calling: [...s.calling, id] }));
    void call()
      .catch((e: unknown) => say("error", `Could not update the release: ${message(e)}`))
      .finally(() => {
        onState((s) => ({ ...s, calling: s.calling.filter((c) => c !== id) }));
        onChanged();
      });
  };
  const move = (t: Task, to: number | null) =>
    void client
      .setTaskRelease(t.id, to)
      .catch((e: unknown) => say("error", `Could not update ${t.name}: ${message(e)}`))
      .finally(onTaskChanged);

  const editing = st.form;
  const form = editing && (editing.kind === "new" ? create : edit);
  return (
    <div className="releases">
      <div className="releases-toolbar">
        {editing && form ? (
          <div className="release-form">
            <Form
              key={editing.opened}
              form={form}
              today={formatDay(day)}
              values={editing.values}
              onValues={(values) => onState((s) => ({ ...s, form: s.form && { ...s.form, values } }))}
              onSubmit={async (submit, values) => {
                const params: Record<string, string> = editing.kind === "new" ? { product } : { id: String(editing.release.id) };
                await client.submitForm(form, submit, params, values);
                set({ form: null });
                onChanged();
              }}
              onCancel={() => set({ form: null })}
            />
          </div>
        ) : (
          <Button variant="primary" onClick={() => open({ kind: "new" })}>
            New release
          </Button>
        )}
      </div>
      {releases.length === 0 && <p className="low small">No releases</p>}
      <div className="releases-cards">
        {orderedReleases(releases).map((r) => {
          const calling = st.calling.includes(r.id);
          return (
            <article key={r.id} className="page-card release-card" aria-label={r.version}>
              <div className="release-title">
                <h2>{r.version}</h2>
                <span className="low small">{`${r.start} to ${r.target}`}</span>
              </div>
              {/* Only this release's tasks count, whatever product the list holds. */}
              <ReleaseBars release={r} progress={releaseProgress(r, tasks, day)} />
              <div className="release-actions">
                {r.released_at === null ? (
                  <Button disabled={calling} onClick={() => change(r.id, () => client.updateRelease(r.id, { released: true }))}>
                    Mark released
                  </Button>
                ) : (
                  <>
                    <span className="low small">{`Released ${formatDay(localDay(r.released_at))}`}</span>
                    <Button
                      className="ghost"
                      disabled={calling}
                      onClick={() => change(r.id, () => client.updateRelease(r.id, { released: false }))}
                    >
                      Unrelease
                    </Button>
                  </>
                )}
                <Button className="ghost" onClick={() => open({ kind: "edit", release: r })}>
                  Edit
                </Button>
                {st.confirmDelete === r.id ? (
                  <>
                    <span className="danger small">{`Delete ${r.version}? Its tasks go to the backlog`}</span>
                    <Button
                      variant="danger"
                      disabled={calling}
                      onClick={() => {
                        set({ confirmDelete: null });
                        change(r.id, () => client.deleteRelease(r.id));
                      }}
                    >
                      Confirm
                    </Button>
                    <Button className="ghost" onClick={() => set({ confirmDelete: null })}>
                      Cancel
                    </Button>
                  </>
                ) : (
                  <Button variant="quiet-danger" disabled={calling} onClick={() => set({ confirmDelete: r.id })}>
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
          );
        })}
      </div>
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
