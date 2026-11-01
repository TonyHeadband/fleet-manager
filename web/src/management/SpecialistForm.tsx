import { useId, useState, type ReactNode } from "react";
import type { DefInfo } from "../api/generated/DefInfo";
import type { Effort } from "../api/generated/Effort";
import type { PermissionMode } from "../api/generated/PermissionMode";
import { Markdown } from "../chat/Markdown";
import { Avatar } from "../ui/Avatar";
import { Button } from "../ui/Button";
import { Chip } from "../ui/Chip";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { ErrorLine } from "../ui/ErrorLine";
import { SaveBar } from "../ui/SaveBar";
import { Section } from "../ui/Section";
import { Select } from "../ui/Select";
import { defDirty, type DefinitionsApi } from "./definitions";
import { fieldOfError, parseDef, pruneDef, writeDef, type DefDraft, type DefField } from "./frontmatter";

// A specialist's or orchestrator's file as a form. The hook's text draft is the one source of truth: the fields are read
// from it and every change writes the whole file back, replacing only the keys that changed.

const MODES: { value: PermissionMode | ""; label: string }[] = [
  { value: "", label: "Default" },
  { value: "auto", label: "Auto" },
  { value: "acceptEdits", label: "Accept edits" },
  { value: "manual", label: "Manual" },
  { value: "dontAsk", label: "Don't ask" },
];
const ROLES = [
  { value: "specialist", label: "Specialist" },
  { value: "orchestrator", label: "Orchestrator" },
];
const EFFORTS: { value: Effort; label: string }[] = [
  { value: "low", label: "Low" },
  { value: "medium", label: "Medium" },
  { value: "high", label: "High" },
];

interface Props {
  persona: string;
  kind: "orchestrator" | "specialist";
  name: string;
  info: DefInfo | undefined;
  defs: DefinitionsApi;
  /** The persona's usable skills, which Add skill offers. */
  skills: string[];
  /** The role cannot be changed here: the tab it is on fixes it. */
  roleFixed: boolean;
}

function Field(props: { label: string; wide?: boolean; error: string | null; children: (id: string) => ReactNode }) {
  const id = useId();
  return (
    <div className={props.wide ? "form-field wide" : "form-field"}>
      <label className="mono-label caption" htmlFor={id}>
        {props.label}
      </label>
      {props.children(id)}
      {props.error !== null && <ErrorLine text={props.error} />}
    </div>
  );
}

export function SpecialistForm({ persona, kind, name, info, defs, skills, roleFixed }: Props) {
  const key = defs.key(kind, name);
  const draft = defs.st.drafts[key];
  // A new definition has a draft and no file until its first Save: it reads as an override not saved yet.
  const file = defs.st.files[key] ?? (draft === undefined ? undefined : { kind, name, text: "", builtin: true });
  const error = defs.st.errors[key];
  const saved = info?.nickname ?? "";

  // The nickname is not in the file: it has its own draft, seeded from the list and dropped with the definition.
  const identity = `${persona}/${kind}/${name}`;
  const [nick, setNick] = useState({ identity, text: saved });
  const [nickError, setNickError] = useState<string | null>(null);
  const [preview, setPreview] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const [roleError, setRoleError] = useState<string | null>(null);
  if (nick.identity !== identity) {
    setNick({ identity, text: saved });
    setNickError(null);
    setRoleError(null);
  }
  const nickText = nick.identity === identity ? nick.text : saved;

  if (!file) return null;
  const text = draft ?? file.text;
  const editable = draft !== undefined;
  const parsed = parseDef(text);
  const change = (fields: Partial<DefDraft["fields"]>, body = parsed.body) =>
    defs.edit(key, writeDef(text, { fields: { ...parsed.fields, ...fields }, body }, true));

  const field: DefField | null = error === undefined ? null : fieldOfError(error);
  const under = (f: DefField) => (field === f && error !== undefined ? error : null);
  const fileDirty = editable && defDirty(draft, file);
  const nickDirty = nickText.trim() !== saved;

  const save = async () => {
    setNickError(null);
    if (fileDirty) {
      // Keys a field emptied while editing leave the file now; the save reads the draft as it stands.
      defs.edit(key, pruneDef(text));
      defs.save(kind, name);
    }
    if (!nickDirty) return;
    try {
      await defs.setNickname(name, nickText.trim());
    } catch (e) {
      setNickError(`Could not save the nickname: ${e instanceof Error ? e.message : String(e)}`);
    }
  };
  const revert = () => {
    if (editable) defs.revert(key);
    setNick({ identity, text: saved });
    setNickError(null);
  };

  // Only the persona's own file can move, and not with unsaved changes: the move works on the file as saved.
  const ownFile = info !== undefined && (info.overridden || !info.builtin);
  const move = async (to: string) => {
    setRoleError(null);
    try {
      await defs.move(kind, name, to as typeof kind);
    } catch (e) {
      setRoleError(e instanceof Error ? e.message : String(e));
    }
  };

  const { fields } = parsed;

  return (
    <div className="specialist-form">
      <Section title="Identity">
        <div className="form-body form-identity">
          <Avatar kind="specialist" seed={name} title={name} />
          <div className="form-fields">
            <Field label="Name" error={null}>
              {(id) => <input id={id} readOnly value={name} />}
            </Field>
            <Field label="Nickname" error={nickError}>
              {(id) => (
                <input
                  id={id}
                  value={nickText}
                  maxLength={32}
                  spellCheck={false}
                  placeholder="none"
                  onChange={(e) => {
                    setNick({ identity, text: e.target.value });
                    setNickError(null);
                  }}
                />
              )}
            </Field>
            <div className="form-field wide">
              <span className="mono-label caption">Role</span>
              {roleFixed || !ownFile ? (
                <span className="form-role" data-fixed={roleFixed || undefined}>
                  {kind === "orchestrator" ? "Orchestrator" : "Specialist"}
                </span>
              ) : (
                <Select
                  label="Role"
                  value={kind}
                  options={ROLES}
                  disabled={fileDirty || nickDirty || defs.st.saving !== null}
                  title={fileDirty || nickDirty ? "Save or revert first" : undefined}
                  onChange={(to) => void move(to)}
                />
              )}
              {roleError !== null && <ErrorLine text={roleError} />}
            </div>
          </div>
        </div>
      </Section>

      <Section title="Settings">
        <div className="form-body">
          <Field label="Summary" error={under("summary")}>
            {(id) => <input id={id} readOnly={!editable} value={fields.summary} onChange={(e) => change({ summary: e.target.value })} />}
          </Field>
          <div className="form-field">
            <span className="mono-label caption">Skills</span>
            <div className="form-chips">
              {fields.skills.map((s) => (
                <span key={s} className="skill-chip" data-tone={skills.includes(s) ? undefined : "danger"}>
                  <Chip
                    selected={false}
                    disabled={!editable}
                    title={skills.includes(s) ? undefined : "Not one of this persona's skills"}
                    onClick={() => change({ skills: fields.skills.filter((x) => x !== s) })}
                  >
                    {s} ×
                  </Chip>
                </span>
              ))}
              {editable && skills.some((s) => !fields.skills.includes(s)) && (
                <Select
                  label="Add skill"
                  value=""
                  placeholder="+ Add skill"
                  options={skills.filter((s) => !fields.skills.includes(s)).map((s) => ({ value: s, label: s }))}
                  onChange={(s) => change({ skills: [...fields.skills, s] })}
                />
              )}
            </div>
            {under("skills") !== null && <ErrorLine text={under("skills") ?? ""} />}
          </div>
          <div className="form-settings">
            <Field label="Model" error={under("model")}>
              {(id) => <input id={id} readOnly={!editable} value={fields.model ?? ""} onChange={(e) => change({ model: e.target.value || null })} />}
            </Field>
            <div className="form-field">
              <span className="mono-label caption">Mode</span>
              <Select
                label="Mode"
                value={fields.mode ?? ""}
                options={MODES}
                disabled={!editable}
                onChange={(m) => change({ mode: (m || null) as PermissionMode | null })}
              />
              {under("mode") !== null && <ErrorLine text={under("mode") ?? ""} />}
            </div>
            <div className="form-field">
              <span className="mono-label caption">Effort</span>
              <div className="form-chips">
                {EFFORTS.map((e) => (
                  <Chip key={e.value} selected={fields.effort === e.value} disabled={!editable} onClick={() => change({ effort: fields.effort === e.value ? null : e.value })}>
                    {e.label}
                  </Chip>
                ))}
              </div>
              {under("effort") !== null && <ErrorLine text={under("effort") ?? ""} />}
            </div>
          </div>
        </div>
      </Section>

      <Section
        title="Instructions"
        aside={
          <div className="form-chips">
            <Chip selected={!preview} onClick={() => setPreview(false)}>
              Edit
            </Chip>
            <Chip selected={preview} onClick={() => setPreview(true)}>
              Preview
            </Chip>
          </div>
        }
      >
        <div className="form-body">
          {preview ? (
            <div className="form-preview">
              <Markdown text={parsed.body} />
            </div>
          ) : (
            <div className="form-field">
              <textarea
                aria-label="Instructions"
                rows={16}
                readOnly={!editable}
                value={parsed.body}
                onChange={(e) => change({}, e.target.value)}
              />
            </div>
          )}
        </div>
      </Section>

      {field === null && error !== undefined && <ErrorLine text={error} />}
      <div className="form-bar">
        {!editable && (
          <Button variant="primary" onClick={() => defs.override(key)}>
            Override
          </Button>
        )}
        <SaveBar
          dirty={fileDirty || nickDirty}
          busy={defs.st.saving !== null}
          onSave={save}
          onRevert={revert}
          note={editable ? undefined : "Built in: Override saves an editable copy as your own file."}
        />
        {info?.overridden && !file.builtin && (
          <Button variant="quiet-danger" onClick={() => setConfirm(true)}>
            Reset to built-in
          </Button>
        )}
      </div>
      {confirm && (
        <ConfirmDialog
          title={`Reset ${name} to built-in?`}
          body="Your copy is deleted and the built-in applies again to agents started after this."
          go="Reset"
          onCancel={() => setConfirm(false)}
          onGo={() => {
            setConfirm(false);
            defs.reset(kind, name);
          }}
        />
      )}
    </div>
  );
}
