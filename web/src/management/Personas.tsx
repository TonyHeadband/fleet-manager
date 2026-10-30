import { useEffect, useState } from "react";
import type { Persona } from "../api/generated/Persona";
import { CaptionField } from "../personas/logins";
import { CHECK_MS } from "../startup/PersonaSetup";
import { Button } from "../ui/Button";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { ErrorLine } from "../ui/ErrorLine";
import { Mark } from "../ui/icons";
import { ListDetail } from "../ui/ListDetail";
import { ListRow } from "../ui/ListRow";
import { settled, type PersonasApi } from "./personas";

// A port of crates/fleet-gui/src/views/management/personas.rs: the personas on the left, the selected one on the
// right, and app/setup.rs's selection, checks and Delete dialog.

type State = "ok" | "warn" | "draft";

function stateOf(p: Persona): State {
  if (!p.ready) return "draft";
  return [p.claude, p.github].some((s) => s && !s.ok) ? "warn" : "ok";
}

/** egui's `row_label`: the text a row is named by; the mark itself is drawn. */
const rowLabel = (p: Persona) => `${p.name} ${{ ok: "✓", warn: "⚠", draft: "draft" }[stateOf(p)]}`;

function StateMark({ p }: { p: Persona }) {
  switch (stateOf(p)) {
    case "ok":
      return <Mark name="ok" label="Both logins work" />;
    case "warn":
      return <Mark name="warn" label="A login needs attention" />;
    case "draft":
      return <span className="low">draft</span>;
  }
}

/** egui's `changed`: the edit is not blank and differs from what is saved. */
export const changed = (edit: string, saved: string) => edit.trim() !== "" && edit.trim() !== saved;

const tasksText = (n: number) => `${n} ${n === 1 ? "task" : "tasks"}`;

interface Props {
  personas: PersonasApi;
  onNew: () => void;
  onResume: (p: Persona) => void;
}

export function Personas({ personas, onNew, onResume }: Props) {
  const { st, listed, select, check, deselect } = personas;
  const [deleting, setDeleting] = useState<Persona | null>(null);
  const persona = listed?.find((p) => p.id === st.selected) ?? null;

  // egui's `keep_selection`: the first persona when none is selected, or the selected one is gone.
  useEffect(() => {
    if (!listed || persona) return;
    if (listed[0]) select(listed[0]);
    else if (st.selected !== null) deselect();
  }, [listed, persona, select, deselect, st.selected]);

  // egui's `keep_checking`: while a login is not ok the person is logging in elsewhere, so ask again now and then.
  const waiting = persona !== null && !settled(persona) ? persona.id : null;
  useEffect(() => {
    if (waiting === null) return;
    const timer = setInterval(() => check(waiting), CHECK_MS);
    return () => clearInterval(timer);
  }, [waiting, check]);

  if (!listed) {
    return (
      <div className="management-page">
        <p className="note">Loading…</p>
      </div>
    );
  }
  return (
    <>
      <ListDetail
        width="management"
        listLabel="Personas"
        detailLabel={persona?.name ?? "Persona"}
        list={
          <ul>
            {listed.map((p) => (
              <li key={p.id}>
                <ListRow
                  name={rowLabel(p)}
                  selected={p.id === st.selected}
                  onSelect={() => (p.id === st.selected ? check(p.id) : select(p))}
                >
                  <span className="persona-row">
                    <StateMark p={p} />
                    <span className="persona-name">{p.name}</span>
                  </span>
                </ListRow>
              </li>
            ))}
          </ul>
        }
        footer={<Button onClick={onNew}>+ New persona</Button>}
        detail={
          persona === null ? (
            <p className="note">No persona selected.</p>
          ) : (
            <div className="persona-pane">
              <h2>{persona.name}</h2>
              <p className="note">Tasks: {persona.tasks}</p>
              {persona.ready ? (
                <Ready persona={persona} personas={personas} onDelete={() => setDeleting(persona)} />
              ) : (
                <Draft persona={persona} onResume={() => onResume(persona)} onDelete={() => setDeleting(persona)} />
              )}
            </div>
          )
        }
      />
      {deleting && (
        <ConfirmDialog
          title={`Delete ${deleting.name}?`}
          body="Its folder is removed, with its Claude and GitHub logins, and so are its clones of product repos. The repos stay with their products. Clones from before products, and clones holding branches its archived tasks kept, stay where they are."
          go="Delete"
          onCancel={() => setDeleting(null)}
          onGo={() => {
            setDeleting(null);
            void personas.remove(deleting.id);
          }}
        />
      )}
    </>
  );
}

/** egui's `detail` for a ready persona: its fields, the errors, and Delete with its blocker. */
function Ready({ persona, personas, onDelete }: { persona: Persona; personas: PersonasApi; onDelete: () => void }) {
  const { st, update } = personas;
  const identity = changed(st.commitName, persona.commit_name) || changed(st.commitEmail, persona.commit_email);
  const valid = st.commitName.trim() !== "" && st.commitEmail.trim() !== "";
  const empty = persona.tasks === 0;
  return (
    <>
      <div className="persona-block">
        <CaptionField label="Name" value={st.name} onChange={(name) => update((s) => ({ ...s, name }))} />
        <div>
          <Button disabled={!changed(st.name, persona.name)}>Save name</Button>
        </div>
      </div>
      <div className="persona-block">
        <CaptionField label="Commit name" value={st.commitName} onChange={(commitName) => update((s) => ({ ...s, commitName }))} />
        <CaptionField label="Commit email" value={st.commitEmail} onChange={(commitEmail) => update((s) => ({ ...s, commitEmail }))} />
        <div>
          <Button disabled={!(identity && valid)}>Save identity</Button>
        </div>
      </div>
      {[st.error, st.checkError].map((e) => e !== null && <ErrorLine key={e} text={e} />)}
      <div className="persona-actions">
        <Button variant="quiet-danger" disabled={!empty} title={empty ? undefined : "Delete or archive its tasks first"} onClick={onDelete}>
          Delete
        </Button>
        {!empty && <span className="note">Used by {tasksText(persona.tasks)}</span>}
      </div>
    </>
  );
}

/** egui's `draft`: what an unfinished setup was set up with, and Resume or Delete. */
function Draft({ persona, onResume, onDelete }: { persona: Persona; onResume: () => void; onDelete: () => void }) {
  const fields: [string, string][] = [
    ["Commit name", persona.commit_name],
    ["Commit email", persona.commit_email],
    ["GitHub host", persona.host],
  ];
  return (
    <>
      <p className="note">Setup was not finished.</p>
      {fields.map(([label, value]) => (
        <div key={label} className="persona-value">
          <span className="mono-label caption">{label}</span>
          <span>{value}</span>
        </div>
      ))}
      <div className="persona-actions">
        <Button variant="primary" onClick={onResume}>
          Resume
        </Button>
        <Button variant="quiet-danger" onClick={onDelete}>
          Delete
        </Button>
      </div>
    </>
  );
}
