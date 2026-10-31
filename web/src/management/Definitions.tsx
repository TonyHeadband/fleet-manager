import { useId, useState } from "react";
import type { DefFile } from "../api/generated/DefFile";
import type { DefInfo } from "../api/generated/DefInfo";
import type { DefKind } from "../api/generated/DefKind";
import { Button } from "../ui/Button";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { ErrorLine } from "../ui/ErrorLine";
import { Mark } from "../ui/icons";
import { ListDetail } from "../ui/ListDetail";
import { ListRow } from "../ui/ListRow";
import { SaveBar } from "../ui/SaveBar";
import { defDirty, defKey, type DefinitionsApi } from "./definitions";
import { Page } from "../ui/Page";

// A port of crates/fleet-gui/src/views/management/definitions.rs: one kind's definitions, built-in or the user's, with
// the selected one's file. A built-in is read-only until Override copies it into a draft that saves as the user's file.

/** egui's `ROWS`: the editor's height in lines. */
const EDITOR_ROWS = 24;

const KINDS: Record<DefKind, { label: string; folder: string }> = {
  orchestrator: { label: "Orchestrators", folder: "orchestrators" },
  specialist: { label: "Specialists", folder: "specialists" },
  skill: { label: "Skills", folder: "skills" },
  pr_style: { label: "PR styles", folder: "pr-styles" },
};

function tag(d: DefInfo) {
  if (d.overridden) return "overrides built-in";
  return d.builtin ? "built-in" : "custom";
}

/** `auto · high`: what the definition's agents start with; nothing for a file that names neither. */
const modeEffort = (d: DefInfo) => [d.mode, d.effort].filter(Boolean).join(" · ");

export function Definitions({ kind, defs }: { kind: DefKind; defs: DefinitionsApi }) {
  const { st } = defs;
  if (st.list === null) {
    return (
      <Page>
        <p className="note">Loading…</p>
      </Page>
    );
  }
  const listed = st.list.filter((d) => d.kind === kind);
  const selected = st.selected?.kind === kind ? st.selected.name : null;
  return (
    <ListDetail
      width="management"
      listLabel={KINDS[kind].label}
      detailLabel={selected ?? "Definition"}
      list={
        <>
          {listed.length === 0 && <p className="note">None yet.</p>}
          <ul>
            {listed.map((d) => (
              <li key={d.name}>
                <ListRow
                  name={`${d.name} ${tag(d)}${d.error ? " ⚠" : ""}`}
                  selected={d.name === selected}
                  onSelect={() => defs.open(kind, d.name)}
                >
                  <span className="definition-row">
                    <span className="definition-row-head">
                      <span className="list-row-name">{d.name}</span>
                      <span className="low">{tag(d)}</span>
                    </span>
                    {modeEffort(d) && <span className="low">{modeEffort(d)}</span>}
                    {d.error && (
                      <span className="definition-row-error">
                        <Mark name="warn" label="Cannot be used" />
                        <span>{d.error}</span>
                      </span>
                    )}
                  </span>
                </ListRow>
              </li>
            ))}
          </ul>
        </>
      }
      detail={
        selected === null ? (
          <p className="note">No definition selected.</p>
        ) : (
          <Editor kind={kind} name={selected} info={listed.find((d) => d.name === selected)} defs={defs} />
        )
      }
    />
  );
}

/** egui's `editor`: read-only with Override for a built-in, else the draft with Save / Revert and, over a built-in,
 * Reset to built-in. The last error stays under it, and the draft with it. */
function Editor({ kind, name, info, defs }: { kind: DefKind; name: string; info: DefInfo | undefined; defs: DefinitionsApi }) {
  const captionId = useId();
  const [confirm, setConfirm] = useState(false);
  const key = defKey(kind, name);
  const file: DefFile | undefined = defs.st.files[key];
  const draft = defs.st.drafts[key];
  const error = defs.st.errors[key];

  const body = () => {
    if (!file) return error !== undefined ? <ErrorLine text={error} /> : <p className="note">Loading…</p>;
    if (draft === undefined)
      return (
        <>
          <textarea readOnly rows={EDITOR_ROWS} aria-labelledby={captionId} value={file.text} />
          <div className="definition-actions">
            <Button variant="primary" onClick={() => defs.override(key)}>
              Override
            </Button>
            <span className="note">Built in: Override saves an editable copy as your own file.</span>
          </div>
        </>
      );
    return (
      <>
        <textarea rows={EDITOR_ROWS} aria-labelledby={captionId} value={draft} onChange={(e) => defs.edit(key, e.target.value)} />
        {error !== undefined && <ErrorLine text={error} />}
        <SaveBar
          dirty={defDirty(draft, file)}
          busy={defs.st.saving !== null}
          onSave={() => defs.save(kind, name)}
          onRevert={() => defs.revert(key)}
        />
        {info?.overridden && !file.builtin && (
          <div className="definition-actions">
            <Button variant="quiet-danger" onClick={() => setConfirm(true)}>
              Reset to built-in
            </Button>
          </div>
        )}
      </>
    );
  };

  return (
    <div className="definition-editor">
      <h3 className="mono-label caption" id={captionId}>
        {KINDS[kind].folder}/{name}
      </h3>
      {info?.error && <ErrorLine text={info.error} />}
      {body()}
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
