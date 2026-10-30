import { useId } from "react";
import type { LoginState } from "../api/generated/LoginState";
import type { Native } from "../native/native";
import { CopyLine, ToolProblem } from "../startup/Requirements";
import { Button } from "../ui/Button";
import "./logins.css";

// egui's login pieces (components/persona_setup.rs), shared by the setup wizard and Management's Personas pane.

/** The CLI behind a login cannot be used (missing, too old, will not start): logging in cannot help until it is fixed. */
export const needsFix = (s: LoginState | null) => s?.tool != null && s.tool.kind !== "not-logged-in";

/** A persona that is only logged out has no fix, and may log in. */
export const canLogIn = (s: LoginState | null) => !needsFix(s);

/** A login's title row, with its button on the right. */
export function LoginHead({ name, action }: { name: string; action?: { label: string; onClick: () => void } | null }) {
  return (
    <div className="login-head">
      <span className="login-name">{name}</span>
      {action && <Button onClick={action.onClick}>{action.label}</Button>}
    </div>
  );
}

/** ✓ account / ✗ detail; a CLI that cannot be used shows as its tool error, with the command to copy and the link. */
export function LoginStatus({ state, native }: { state: LoginState | null; native: Native | null }) {
  if (state?.tool && needsFix(state)) return <ToolProblem error={state.tool} native={native} />;
  const [tone, text] =
    state == null
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

/** egui's `command_hint`: the login's script, to run by hand because no terminal was found (or this is a browser). */
export function ByHand({ script, native }: { script: string | undefined; native: Native | null }) {
  if (script === undefined) return null;
  return (
    <>
      <p className="note">No terminal found. Run this in one:</p>
      <CopyLine text={script} native={native} />
    </>
  );
}

/** egui's `pat_input`: the masked token field with Save token, usable only where the app can hand it to gh. */
export function PatInput(props: { value: string; onChange: (v: string) => void; busy: boolean; usable: boolean; onSave: () => void }) {
  const { value, onChange, busy, usable, onSave } = props;
  const id = useId();
  const canSave = usable && !busy && value.trim() !== "";
  return (
    <div>
      <label className="mono-label caption" htmlFor={id}>
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

/** egui's captioned single-line field. */
export function CaptionField(props: { label: string; value: string; onChange: (v: string) => void; hint?: string; maxLength?: number }) {
  const { label, value, onChange, hint, maxLength } = props;
  const id = useId();
  return (
    <div className="caption-field">
      <label className="mono-label caption" htmlFor={id}>
        {label}
      </label>
      <input id={id} value={value} placeholder={hint} maxLength={maxLength} onChange={(e) => onChange(e.target.value)} spellCheck={false} />
    </div>
  );
}
