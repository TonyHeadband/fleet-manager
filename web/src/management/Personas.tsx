import { useEffect, useId, useState } from "react";
import type { DefInfo } from "../api/generated/DefInfo";
import type { GithubAuth } from "../api/generated/GithubAuth";
import type { Persona } from "../api/generated/Persona";
import type { Native } from "../native/native";
import { ByHand, CaptionField, canLogIn, LoginHead, LoginStatus, PatInput } from "../personas/logins";
import { CHECK_MS } from "../startup/PersonaSetup";
import { Button } from "../ui/Button";
import { Chip } from "../ui/Chip";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { ErrorLine } from "../ui/ErrorLine";
import { Mark } from "../ui/icons";
import { ListDetail } from "../ui/ListDetail";
import { ListRow } from "../ui/ListRow";
import { usable } from "./definitions";
import { cleaned, sameNicknames, settled, type PersonasApi } from "./personas";

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
  /** The definitions list, for the nickname fields; null until it answers. */
  defs: DefInfo[] | null;
  native: Native | null;
}

export function Personas({ personas, onNew, onResume, defs, native }: Props) {
  const { st, listed, select, check, deselect } = personas;
  const [deleting, setDeleting] = useState<Persona | null>(null);
  const persona = listed?.find((p) => p.id === st.selected) ?? null;

  const loadMeta = personas.loadMeta;
  useEffect(loadMeta, [loadMeta]);

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
      <div className="tabbed-page">
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
                <Ready persona={persona} personas={personas} defs={defs} native={native} onDelete={() => setDeleting(persona)} />
              ) : (
                <Draft persona={persona} personas={personas} onResume={() => onResume(persona)} onDelete={() => setDeleting(persona)} />
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
function Ready(props: { persona: Persona; personas: PersonasApi; defs: DefInfo[] | null; native: Native | null; onDelete: () => void }) {
  const { persona, personas, defs, native, onDelete } = props;
  const { st, update } = personas;
  const identity = changed(st.commitName, persona.commit_name) || changed(st.commitEmail, persona.commit_email);
  const valid = st.commitName.trim() !== "" && st.commitEmail.trim() !== "";
  const empty = persona.tasks === 0;
  return (
    <>
      <div className="persona-block">
        <CaptionField label="Name" value={st.name} onChange={(name) => update((s) => ({ ...s, name }))} />
        <div>
          <Button disabled={!changed(st.name, persona.name)} onClick={() => personas.rename(persona)}>
            Save name
          </Button>
        </div>
      </div>
      <div className="persona-block">
        <CaptionField label="Commit name" value={st.commitName} onChange={(commitName) => update((s) => ({ ...s, commitName }))} />
        <CaptionField label="Commit email" value={st.commitEmail} onChange={(commitEmail) => update((s) => ({ ...s, commitEmail }))} />
        <div>
          <Button disabled={!(identity && valid)} onClick={() => personas.saveIdentity(persona)}>
            Save identity
          </Button>
        </div>
      </div>
      <Nicknames persona={persona} personas={personas} defs={defs} />
      <div className="login-row">
        <LoginHead
          name="Claude"
          action={canLogIn(persona.claude) ? { label: "Re-login to Claude", onClick: () => personas.relogin(persona, "claude") } : null}
        />
        <LoginStatus state={persona.claude} native={native} />
        <ByHand script={st.scripts.claude} native={native} />
      </div>
      <div className="login-row">
        <LoginHead
          name={`GitHub · ${persona.host}`}
          action={canLogIn(persona.github) ? { label: "Re-login to GitHub", onClick: () => personas.relogin(persona, "gh") } : null}
        />
        <LoginStatus state={persona.github} native={native} />
        <span className="mono-label caption">GitHub auth</span>
        <div className="chips">
          {AUTHS.map(([auth, label]) => (
            <Chip key={auth} selected={st.githubAuth === auth} onClick={() => update((s) => ({ ...s, githubAuth: auth }))}>
              {label}
            </Chip>
          ))}
        </div>
        {st.githubAuth !== persona.github_auth && <p className="note">Re-login switches to this.</p>}
        {st.patOpen && (
          <PatInput
            value={st.pat}
            onChange={(pat) => update((s) => ({ ...s, pat }))}
            busy={st.patBusy}
            usable={personas.canSavePat}
            onSave={() => personas.submitPat(persona)}
          />
        )}
        <ByHand script={st.scripts.gh} native={native} />
      </div>
      <Errors personas={personas} />
      <div className="persona-actions">
        <Button variant="quiet-danger" disabled={!empty} title={empty ? undefined : "Delete or archive its tasks first"} onClick={onDelete}>
          Delete
        </Button>
        {!empty && <span className="note">Used by {tasksText(persona.tasks)}</span>}
      </div>
    </>
  );
}

const AUTHS: [GithubAuth, string][] = [
  ["gh", "gh login"],
  ["pat", "Personal access token"],
];

/** egui's `nicknames_section`: a field per specialist that loads. One kept for a specialist that no longer loads stays
 * stored, out of sight, until it loads again. */
function Nicknames({ persona, personas, defs }: { persona: Persona; personas: PersonasApi; defs: DefInfo[] | null }) {
  const { st, update } = personas;
  const names = defs ? usable(defs, "specialist") : [];
  const dirty = !sameNicknames(cleaned(st.nicknames), persona.nicknames);
  return (
    <div className="persona-block">
      <span className="mono-label caption">Specialist nicknames</span>
      <p className="note">Shown in the app wherever this persona's tasks name a specialist. Agents and logs keep the real names.</p>
      {defs === null ? (
        <p className="note">The specialists have not loaded.</p>
      ) : (
        <>
          <div className="nicknames">
            {names.map((name) => (
              <Nickname
                key={name}
                name={name}
                value={st.nicknames[name] ?? ""}
                onChange={(v) => update((s) => ({ ...s, nicknames: { ...s.nicknames, [name]: v } }))}
              />
            ))}
          </div>
          <div>
            <Button disabled={!dirty} onClick={() => personas.saveNicknames(persona)}>
              Save nicknames
            </Button>
          </div>
        </>
      )}
    </div>
  );
}

function Nickname({ name, value, onChange }: { name: string; value: string; onChange: (v: string) => void }) {
  const id = useId();
  return (
    <>
      <label className="nickname-name" htmlFor={id}>
        {name}
      </label>
      <input id={id} value={value} placeholder="none" maxLength={32} onChange={(e) => onChange(e.target.value)} spellCheck={false} />
    </>
  );
}

/** egui's `draft`: what an unfinished setup was set up with, and Resume or Delete. */
/** The pane's last error and the check's, under either pane: a refused delete of a draft says why too. */
function Errors({ personas: { st } }: { personas: PersonasApi }) {
  return [st.error, st.checkError].map((e) => e !== null && <ErrorLine key={e} text={e} />);
}

function Draft(props: { persona: Persona; personas: PersonasApi; onResume: () => void; onDelete: () => void }) {
  const { persona, personas, onResume, onDelete } = props;
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
      <Errors personas={personas} />
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
