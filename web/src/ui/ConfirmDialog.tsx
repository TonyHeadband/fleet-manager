import { useEffect, useId } from "react";
import { Button } from "./Button";

interface Props {
  title: string;
  body: string;
  /** The danger button's label. */
  go: string;
  onGo: () => void;
  onCancel: () => void;
}

/** Lite's `confirm_dialog`: a question over the window, the focus on Cancel. Escape and a click outside cancel. */
export function ConfirmDialog({ title, body, go, onGo, onCancel }: Props) {
  const titleId = useId();
  useEffect(() => {
    const key = (e: KeyboardEvent) => e.key === "Escape" && onCancel();
    document.addEventListener("keydown", key);
    return () => document.removeEventListener("keydown", key);
  }, [onCancel]);
  return (
    <div className="dialog-backdrop" onClick={(e) => e.target === e.currentTarget && onCancel()}>
      <div className="dialog" role="dialog" aria-modal="true" aria-labelledby={titleId}>
        <h2 id={titleId}>{title}</h2>
        <p className="dialog-body">{body}</p>
        <div className="form-actions">
          <Button autoFocus onClick={onCancel}>
            Cancel
          </Button>
          <Button variant="danger" onClick={onGo}>
            {go}
          </Button>
        </div>
      </div>
    </div>
  );
}
