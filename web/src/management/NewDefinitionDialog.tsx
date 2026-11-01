import { useId, useState } from "react";
import type { DefInfo } from "../api/generated/DefInfo";
import { Button } from "../ui/Button";
import { ErrorLine } from "../ui/ErrorLine";
import { Modal } from "../ui/Modal";
import { Select } from "../ui/Select";
import { ROLES } from "./SpecialistForm";

type Role = "orchestrator" | "specialist";

/** Why `name` cannot be a new definition of this persona, or null. The daemon's own check follows on Save. */
export function refusal(name: string, list: DefInfo[], role: Role): string | null {
  // The daemon's is_safe_dir_name: the name is a file name.
  if (!/^[A-Za-z0-9_][A-Za-z0-9._-]*$/.test(name)) {
    return "Use letters, digits, '.', '_' or '-', and do not start with '.' or '-'.";
  }
  const same = list.filter((d) => (d.kind === "specialist" || d.kind === "orchestrator") && d.name === name);
  if (same.some((d) => d.kind === role && d.builtin && !d.overridden)) return "That's a built-in: open it and Override.";
  const other = same.find((d) => d.kind !== role && d.builtin);
  if (other) return `That's the built-in ${other.kind} ${name}; choose another name.`;
  if (same.length > 0) return `This persona already has ${name}.`;
  return null;
}

interface Props {
  list: DefInfo[];
  /** The tab's kind: the role it starts as. */
  kind: Role;
  /** The tab fixes the role. */
  roleFixed: boolean;
  /** OK saves nothing: the caller opens an unsaved draft. */
  onCreate: (kind: Role, name: string) => void;
  onClose: () => void;
}

export function NewDefinitionDialog({ list, kind, roleFixed, onCreate, onClose }: Props) {
  const titleId = useId();
  const nameId = useId();
  const [name, setName] = useState("");
  const [role, setRole] = useState<Role>(kind);
  const error = name === "" ? null : refusal(name, list, role);
  const ok = name !== "" && error === null;
  return (
    <Modal labelledBy={titleId} onClose={onClose}>
      <h2 id={titleId}>New {role}</h2>
      <form
        className="new-definition"
        onSubmit={(e) => {
          e.preventDefault();
          if (ok) onCreate(role, name);
        }}
      >
        <div className="form-field">
          <label className="mono-label caption" htmlFor={nameId}>
            Name
          </label>
          <input id={nameId} autoFocus value={name} spellCheck={false} onChange={(e) => setName(e.target.value)} />
          {error !== null && <ErrorLine text={error} />}
        </div>
        <div className="form-field">
          <span className="mono-label caption">Role</span>
          <Select label="Role" value={role} options={ROLES} disabled={roleFixed} onChange={(r) => setRole(r as Role)} />
        </div>
        <div className="form-actions">
          <Button variant="primary" type="submit" disabled={!ok}>
            OK
          </Button>
          <Button onClick={onClose}>Cancel</Button>
        </div>
      </form>
    </Modal>
  );
}
