import { useId, useState } from "react";
import type { Native } from "../native/native";
import { Button } from "../ui/Button";

interface Props {
  native: Native;
  heading: string;
  defaults: { root: string; personas: string };
  error: string | null;
  onNext: (root: string, personas: string) => void;
}

/** egui's first-run step: where tasks and personas live, asked before the daemon is started. */
export function FirstRun({ native, heading, defaults, error, onNext }: Props) {
  const [root, setRoot] = useState(defaults.root);
  const [personas, setPersonas] = useState(defaults.personas);
  return (
    <section className="startup-card wizard">
      <h2>{heading}</h2>
      <p>Where should tasks live? Each task gets its own folder here.</p>
      {/* A form, so Enter in a field is Next. */}
      <form
        onSubmit={(e) => {
          e.preventDefault();
          onNext(root, personas);
        }}
      >
        <FolderRow caption="Tasks folder" value={root} onChange={setRoot} native={native} />
        <FolderRow caption="Personas folder" value={personas} onChange={setPersonas} native={native} />
        <p className="note">Each persona gets its own Claude and GitHub login here. Fleet Manager never reads the tokens.</p>
        {error && <p className="danger">{error}</p>}
        <div className="actions">
          <Button type="submit" variant="primary">
            Next →
          </Button>
        </div>
      </form>
    </section>
  );
}

/** A folder field with Browse…, which only fills the field: Next still creates the folder, so a pick can be edited. */
export function FolderRow({
  caption,
  value,
  onChange,
  native,
}: {
  caption: string;
  value: string;
  onChange: (v: string) => void;
  native: Native;
}) {
  const id = useId();
  return (
    <div className="folder-row">
      <label className="caption" htmlFor={id}>
        {caption}
      </label>
      <div className="folder-input">
        <input id={id} value={value} onChange={(e) => onChange(e.target.value)} spellCheck={false} />
        <Button onClick={() => void native.pickFolder(value.trim() || null).then((p) => p !== null && onChange(p))}>Browse…</Button>
      </div>
    </div>
  );
}
