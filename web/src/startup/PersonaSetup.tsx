import { useCallback, useEffect, useRef, useState } from "react";
import type { FleetClient } from "../api/client";
import type { GithubAuth } from "../api/generated/GithubAuth";
import type { LoginState } from "../api/generated/LoginState";
import type { NewPersona } from "../api/generated/NewPersona";
import type { Persona } from "../api/generated/Persona";
import type { PersonaList } from "../api/generated/PersonaList";
import { claudeCommand, claudeLoginScript, ghLoginScript, ghProgram } from "../api/login";
import type { Native } from "../native/native";
import { Button } from "../ui/Button";
import { Check } from "../ui/Check";
import { Chip } from "../ui/Chip";
import { wizardHeading, type WizardSteps } from "./boot";
import { ByHand, CaptionField as TextField, canLogIn, LoginHead, LoginStatus, needsFix, PatInput } from "../personas/logins";

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
  /** egui's error toast: a refused draft delete, said after the setup may have closed. */
  say: (text: string) => void;
  /**
   * egui's `SetupMode`: the first-run wizard (no way out: the board needs a ready persona), Management's "+ New
   * persona", or Resume of a draft from Management (its Log in step; the draft keeps its own Delete there).
   */
  mode?: SetupMode;
  /** Outside the wizard: the setup closes. */
  onClose?: () => void;
}

export type SetupMode = "wizard" | "new" | "resume";

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
export function PersonaSetup({
  client,
  native,
  local,
  steps,
  stillMissing,
  draft,
  onFinish,
  onChanged,
  say,
  mode = "wizard",
  onClose,
}: Props) {
  const wizard = mode === "wizard";
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
  const deleteDraft = (p: Persona) => {
    current.current = null;
    setPersona(null);
    setPat("");
    setCheckError(null);
    setScripts({});
    setError(null);
    client.deletePersona(p.id).then(onChanged, (e: unknown) => {
      say(`Could not delete the persona: ${message(e)}`);
      onChanged();
    });
  };

  // egui's SetupAction::Cancel: a draft made here is deleted; a resumed one belongs to Management, which has its Delete.
  const cancel = () => {
    if (persona && !persona.ready && mode !== "resume") deleteDraft(persona);
    if (!wizard) onClose?.();
  };

  // Escape and a click outside cancel, except in the wizard, which has no way out.
  useEffect(() => {
    if (wizard) return;
    const key = (e: KeyboardEvent) => e.key === "Escape" && cancelRef.current();
    document.addEventListener("keydown", key);
    return () => document.removeEventListener("keydown", key);
  }, [wizard]);
  const cancelRef = useRef(cancel);
  cancelRef.current = cancel;

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
    // On the press, not the click: a text selection dragged out of the card must not close it.
    <div className="dialog-backdrop" onMouseDown={(e) => !wizard && e.target === e.currentTarget && cancel()}>
      <section className="startup-card wizard persona-setup" role="dialog" aria-modal="true">
        {persona === null ? (
          <Identity
            heading={wizard ? wizardHeading(steps, "identity", "Your first persona") : "New persona"}
            fields={fields}
            onFields={setFields}
            busy={busy}
            error={error}
            onNext={create}
            onCancel={wizard ? null : cancel}
          />
        ) : (
          <>
            <h2>{wizard ? wizardHeading(steps, "login", "Log in") : "Log in"}</h2>
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
            {wizard && stillMissing.length > 0 && (
              <p className="note">Still missing: {stillMissing.join(", ")} — install them from Settings → Relaunch setup wizard</p>
            )}
            {wizard && native && (
              <Check checked={menuShortcut} onChange={setMenuShortcut}>
                Add an app menu shortcut
              </Check>
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
              {!persona.ready && <Button onClick={cancel}>Cancel</Button>}
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
  /** Outside the wizard, Identity has a way out. */
  onCancel: (() => void) | null;
}) {
  const { heading, fields: d, onFields, busy, error, onNext, onCancel } = props;
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
        {onCancel && <Button onClick={onCancel}>Cancel</Button>}
      </div>
    </>
  );
}

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
      <LoginHead name={name} action={onLogin && canLogIn(state) ? { label: "Log in…", onClick: onLogin } : null} />
      {children}
      <LoginStatus state={state} native={native} />
      <ByHand script={script} native={native} />
    </div>
  );
}
