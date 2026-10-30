import { useCallback, useEffect, useId, useRef, useState } from "react";
import type { FleetClient } from "../api/client";
import type { GithubAuth } from "../api/generated/GithubAuth";
import type { LoginState } from "../api/generated/LoginState";
import type { NewPersona } from "../api/generated/NewPersona";
import type { Persona } from "../api/generated/Persona";
import type { PersonaList } from "../api/generated/PersonaList";
import { claudeCommand, claudeLoginScript, ghLoginScript, ghProgram } from "../api/login";
import type { Native } from "../native/native";
import { Button } from "../ui/Button";
import { Chip } from "../ui/Chip";
import { wizardHeading, type WizardSteps } from "./boot";
import { CopyLine, ToolProblem } from "./Requirements";

/** egui's `CHECK_EVERY`: how often the Log in step asks the daemon about the logins. */
export const CHECK_MS = 3000;

type Which = "claude" | "gh";

interface Props {
  client: FleetClient;
  native: Native | null;
  /** The daemon is on this machine: the app opens the login terminals itself. */
  local: boolean;
  steps: WizardSteps;
  /** Required tools the person continued without; the last step names them. */
  stillMissing: string[];
  /** A persona left behind (a crash between the steps): picked up at Log in rather than duplicated. */
  draft: Persona | null;
  onFinish: () => void;
  /** A persona was created or deleted: the list is stale. */
  onChanged: () => void;
}

const EMPTY: NewPersona = { name: "", host: "", commit_name: "", commit_email: "", github_auth: "gh" };

const fieldsOf = (p: Persona): NewPersona => ({
  name: p.name,
  host: p.host,
  commit_name: p.commit_name,
  commit_email: p.commit_email,
  github_auth: p.github_auth,
});

/** The draft as the daemon gets it: trimmed, and an empty host is the placeholder's `github.com`. */
const submitted = (d: NewPersona): NewPersona => ({
  name: d.name.trim(),
  host: d.host.trim() || "github.com",
  commit_name: d.commit_name.trim(),
  commit_email: d.commit_email.trim(),
  github_auth: d.github_auth,
});

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/**
 * egui's persona setup in its wizard mode: over the board, with no way out until a persona is ready, because the
 * board needs one. Identity creates the persona; Log in watches its logins until both work.
 */
export function PersonaSetup({ client, native, local, steps, stillMissing, draft, onFinish, onChanged }: Props) {
  const [fields, setFields] = useState<NewPersona>(draft ? fieldsOf(draft) : EMPTY);
  const [persona, setPersona] = useState<Persona | null>(draft);
  const [meta, setMeta] = useState<PersonaList | null>(null);
  const [pat, setPat] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [checkError, setCheckError] = useState<string | null>(null);
  const [scripts, setScripts] = useState<Partial<Record<Which, string>>>({});
  const [menuShortcut, setMenuShortcut] = useState(false);
  // The persona whose check answers count: a check landing after Cancel is for a persona that is gone.
  const current = useRef<string | null>(draft?.id ?? null);
  const checking = useRef(false);

  useEffect(() => {
    client.personas().then(setMeta, () => {});
  }, [client]);

  const check = useCallback(() => {
    const id = current.current;
    if (id === null || checking.current) return;
    checking.current = true;
    client
      .checkPersona(id)
      .then(
        (p) => {
          if (current.current !== p.id) return;
          setPersona(p);
          setCheckError(null);
        },
        (e: unknown) => current.current === id && setCheckError(`Could not check the logins: ${message(e)}`),
      )
      .finally(() => (checking.current = false));
  }, [client]);

  const id = persona?.id ?? null;
  useEffect(() => {
    if (id === null) return;
    check();
    const timer = setInterval(check, CHECK_MS);
    return () => clearInterval(timer);
  }, [id, check]);

  const create = () => {
    setBusy(true);
    setError(null);
    client
      .createPersona(submitted(fields))
      .then(
        (p) => {
          current.current = p.id;
          setPersona(p);
          onChanged();
        },
        (e: unknown) => setError(message(e)),
      )
      .finally(() => setBusy(false));
  };

  // Back to Identity, the typed values kept; what belonged to the deleted draft goes.
  const cancel = (p: Persona) => {
    current.current = null;
    setPersona(null);
    setPat("");
    setCheckError(null);
    setScripts({});
    setError(null);
    client.deletePersona(p.id).then(onChanged, (e: unknown) => {
      setError(`Could not delete the persona: ${message(e)}`);
      onChanged();
    });
  };

  const login = (p: Persona, which: Which) => {
    setError(null);
    const byHand = (script: string) => setScripts((s) => ({ ...s, [which]: script }));
    if (native && local) {
      native.runLogin(p.id, which).then(
        (r) => r.script !== null && byHand(r.script),
        (e: unknown) => setError(message(e)),
      );
    } else if (meta) {
      // A browser opens no terminal: the script is for the daemon's machine.
      byHand(which === "claude" ? claudeLoginScript(p, claudeCommand(meta), meta.tasks_root) : ghLoginScript(p, ghProgram(meta)));
    }
  };

  const savePat = (p: Persona) => {
    if (!native) return;
    const token = pat;
    setPat("");
    setBusy(true);
    setError(null);
    native
      .savePat(p.id, token)
      .then(check, (e: unknown) => setError(message(e)))
      .finally(() => setBusy(false));
  };

  const finish = () => {
    if (menuShortcut) void native?.addMenuShortcut();
    onFinish();
  };

  return (
    <div className="dialog-backdrop">
      <section className="startup-card wizard persona-setup" role="dialog" aria-modal="true">
        {persona === null ? (
          <Identity
            heading={wizardHeading(steps, "identity", "Your first persona")}
            fields={fields}
            onFields={setFields}
            busy={busy}
            error={error}
            onNext={create}
          />
        ) : (
          <>
            <h2>{wizardHeading(steps, "login", "Log in")}</h2>
            <p className="note">Each opens a terminal running the real CLI. Claude starts in {meta?.tasks_root ?? "…"}.</p>
            <LoginRow name="Claude" state={persona.claude} native={native} script={scripts.claude} onLogin={() => login(persona, "claude")}>
              <p className="note">Trust the folder (second option), then type /login</p>
            </LoginRow>
            <LoginRow
              name={`GitHub · ${persona.host}`}
              state={persona.github}
              native={native}
              script={scripts.gh}
              onLogin={persona.github_auth === "gh" ? () => login(persona, "gh") : null}
            >
              {persona.github_auth === "pat" && !needsFix(persona.github) && (
                <PatInput value={pat} onChange={setPat} busy={busy} usable={native !== null && local} onSave={() => savePat(persona)} />
              )}
            </LoginRow>
            {stillMissing.length > 0 && (
              <p className="note">Still missing: {stillMissing.join(", ")} — install them from Settings → Relaunch setup wizard</p>
            )}
            {native && (
              <label className="check">
                <input type="checkbox" checked={menuShortcut} onChange={(e) => setMenuShortcut(e.target.checked)} />
                Add an app menu shortcut
              </label>
            )}
            {[error, checkError].map(
              (e) =>
                e && (
                  <p key={e} className="danger">
                    {e}
                  </p>
                ),
            )}
            <div className="actions end">
              <Button variant="primary" disabled={!persona.ready} onClick={finish}>
                Finish
              </Button>
              {/* A ready persona may already be the active one: Cancel would delete it from under the app. */}
              {!persona.ready && <Button onClick={() => cancel(persona)}>Cancel</Button>}
              <Button onClick={check}>Check now</Button>
            </div>
          </>
        )}
      </section>
    </div>
  );
}

function Identity(props: {
  heading: string;
  fields: NewPersona;
  onFields: (f: NewPersona) => void;
  busy: boolean;
  error: string | null;
  onNext: () => void;
}) {
  const { heading, fields: d, onFields, busy, error, onNext } = props;
  const set = (k: keyof NewPersona) => (v: string) => onFields({ ...d, [k]: v });
  const filled = [d.name, d.commit_name, d.commit_email].every((f) => f.trim() !== "");
  const auths: [GithubAuth, string][] = [
    ["gh", "gh login"],
    ["pat", "Personal access token"],
  ];
  return (
    <>
      <h2>{heading}</h2>
      <TextField label="Name" value={d.name} onChange={set("name")} />
      <TextField label="Commit name" value={d.commit_name} onChange={set("commit_name")} />
      <TextField label="Commit email" value={d.commit_email} onChange={set("commit_email")} />
      <TextField label="GitHub host" value={d.host} onChange={set("host")} hint="github.com" />
      <div>
        <span className="caption">GitHub auth</span>
        <div className="chips">
          {auths.map(([auth, label]) => (
            <Chip key={auth} selected={d.github_auth === auth} onClick={() => onFields({ ...d, github_auth: auth })}>
              {label}
            </Chip>
          ))}
        </div>
      </div>
      {error && <p className="danger">{error}</p>}
      <div className="actions end">
        <Button variant="primary" disabled={!filled || busy} onClick={onNext}>
          Next
        </Button>
      </div>
    </>
  );
}

function TextField({ label, value, onChange, hint }: { label: string; value: string; onChange: (v: string) => void; hint?: string }) {
  const id = useId();
  return (
    <div>
      <label className="caption" htmlFor={id}>
        {label}
      </label>
      <input id={id} value={value} placeholder={hint} onChange={(e) => onChange(e.target.value)} spellCheck={false} />
    </div>
  );
}

/** The CLI behind a login cannot be used (missing, too old, will not start): logging in cannot help until it is fixed. */
const needsFix = (s: LoginState | null) => s?.tool != null && s.tool.kind !== "not-logged-in";

function LoginRow(props: {
  name: string;
  state: LoginState | null;
  native: Native | null;
  script: string | undefined;
  /** Null: this login has no terminal (a token is typed instead). */
  onLogin: (() => void) | null;
  children?: React.ReactNode;
}) {
  const { name, state, native, script, onLogin, children } = props;
  return (
    <div className="login-row">
      <div className="login-head">
        <span className="login-name">{name}</span>
        {onLogin && !needsFix(state) && <Button onClick={onLogin}>Log in…</Button>}
      </div>
      {children}
      <LoginStatus state={state} native={native} />
      {script !== undefined && (
        <>
          <p className="note">No terminal found. Run this in one:</p>
          <CopyLine text={script} native={native} />
        </>
      )}
    </div>
  );
}

/** ✓ account / ✗ detail; a CLI that cannot be used shows as its tool error, with the command to copy and the link. */
function LoginStatus({ state, native }: { state: LoginState | null; native: Native | null }) {
  if (state?.tool && needsFix(state)) return <ToolProblem error={state.tool} native={native} />;
  const [tone, text] =
    state === null
      ? ["muted", "Not checked yet"]
      : state.ok
        ? ["done", state.account ?? "logged in"]
        : ["attention", state.detail ?? "not logged in"];
  return (
    <p className="login-status">
      <span className="dot" data-tone={tone} />
      {text}
    </p>
  );
}

function PatInput(props: { value: string; onChange: (v: string) => void; busy: boolean; usable: boolean; onSave: () => void }) {
  const { value, onChange, busy, usable, onSave } = props;
  const id = useId();
  const canSave = usable && !busy && value.trim() !== "";
  return (
    <div>
      <label className="caption" htmlFor={id}>
        Personal access token
      </label>
      <div className="folder-input">
        <input id={id} type="password" autoComplete="off" value={value} disabled={!usable} onChange={(e) => onChange(e.target.value)} />
        <Button
          disabled={!canSave}
          onClick={onSave}
          title={usable ? undefined : "Only the Fleet Manager app on the daemon's machine can hand a token to gh"}
        >
          Save token
        </Button>
      </div>
      {/* gh checks a classic token's scopes at login; a fine-grained one only fails later, at the first clone or PR. */}
      <p className="note">
        Fine-grained token: give it the repos this persona works on, with Contents and Pull requests: Read and write (Metadata: Read is
        added for you). A classic token needs the repo and read:org scopes.
      </p>
    </div>
  );
}
