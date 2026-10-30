import { useEffect, useRef } from "react";
import { Button } from "../ui/Button";

interface Props {
  /** The input action's label: the placeholder reads "<label>…". */
  label: string;
  text: string;
  onText: (text: string) => void;
  onSend: (text: string) => void;
  /** Take the cursor when it first draws (the panel just opened on this agent). */
  focus?: boolean;
}

/**
 * Lite's `reply_box`: the side panel's answer box. Enter is a newline; Ctrl/Cmd+Enter or Send sends. The caller
 * clears `text` once the send succeeds, so a failed send keeps the draft.
 */
export function ReplyBox({ label, text, onText, onSend, focus = false }: Props) {
  const box = useRef<HTMLTextAreaElement>(null);
  const ready = text.trim() !== "";
  useEffect(() => {
    if (focus) box.current?.focus();
    // Only on the first draw: a later focus would steal the cursor from wherever the user went.
  }, []);
  return (
    <form
      className="reply"
      onSubmit={(e) => {
        e.preventDefault();
        if (ready) onSend(text);
      }}
    >
      <textarea
        ref={box}
        rows={3}
        placeholder={`${label}…`}
        value={text}
        onChange={(e) => onText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key !== "Enter" || !(e.ctrlKey || e.metaKey)) return;
          e.preventDefault();
          if (ready) onSend(text);
        }}
      />
      <div className="reply-row">
        <span className="reply-hint">Ctrl+Enter</span>
        <Button type="submit" variant="primary" disabled={!ready}>
          Send
        </Button>
      </div>
    </form>
  );
}
