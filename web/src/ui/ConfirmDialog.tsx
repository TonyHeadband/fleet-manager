import { useId } from "react";
import { Button, type ButtonVariant } from "./Button";
import { Modal } from "./Modal";

interface Props {
  title: string;
  body: string;
  /** The danger button's label. */
  go: string;
  onGo: () => void;
  onCancel: () => void;
  /** egui's restart dialog: [Restart anyway] (Default) then [Cancel] (Primary). The others: Cancel, then danger. */
  goVariant?: ButtonVariant;
  cancelVariant?: ButtonVariant;
  goFirst?: boolean;
}

/** Lite's `confirm_dialog`: a question over the window, the focus on Cancel. Escape and a click outside cancel. */
export function ConfirmDialog({ title, body, go, onGo, onCancel, goVariant = "danger", cancelVariant = "default", goFirst = false }: Props) {
  const titleId = useId();
  const goButton = (
    <Button variant={goVariant} onClick={onGo}>
      {go}
    </Button>
  );
  return (
    <Modal labelledBy={titleId} onClose={onCancel}>
      <h2 id={titleId}>{title}</h2>
      <p className="dialog-body">{body}</p>
      <div className="form-actions">
        {goFirst && goButton}
        <Button variant={cancelVariant} autoFocus onClick={onCancel}>
          Cancel
        </Button>
        {!goFirst && goButton}
      </div>
    </Modal>
  );
}
