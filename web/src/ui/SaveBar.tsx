import { Button } from "./Button";

interface Props {
  /** The draft differs from what is saved. */
  dirty: boolean;
  /** Save without a change: a broken file on disk is repaired by saving. */
  force?: boolean;
  /** A save is in flight. */
  busy: boolean;
  onSave: () => void;
  onRevert: () => void;
  note?: string;
}

/** egui's `save_bar`: Save when there is something to save and nothing in flight, Revert while the draft differs. */
export function SaveBar({ dirty, force = false, busy, onSave, onRevert, note = "Applies to agents started after saving." }: Props) {
  return (
    <div className="save-bar">
      <Button variant="primary" disabled={!(dirty || force) || busy} onClick={onSave}>
        Save
      </Button>
      <Button disabled={!dirty} onClick={onRevert}>
        Revert
      </Button>
      <span className="note">{note}</span>
    </div>
  );
}
