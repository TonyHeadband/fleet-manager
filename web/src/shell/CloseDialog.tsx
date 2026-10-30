import { useEffect, useState, type ReactNode } from "react";
import type { Agent } from "../api/generated/Agent";
import type { CloseAnswer, Native } from "../native/native";
import { Button } from "../ui/Button";
import { Check } from "../ui/Check";
import { dialogText, summarize } from "./close";

type Phase = { kind: "idle" } | { kind: "asking"; remember: boolean } | { kind: "stopping" } | { kind: "failed"; message: string };

/**
 * egui's close flow, drawn: the app caught the window's close and decided (fleet-client's `close_action`); this
 * shows its question with live numbers, the stop under way, or why the stop failed. A browser tab has none.
 */
export function CloseDialog({ native, agents }: { native: Native | null; agents: Agent[] }) {
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });

  useEffect(() => {
    if (!native) return;
    void native.setCloseArmed(true);
    const stop = native.onClose((e) =>
      setPhase((p) => {
        // A second request while the dialog is up keeps it as it is, the box ticked or not.
        if (e.phase === "asking") return p.kind === "asking" ? p : { kind: "asking", remember: false };
        if (e.phase === "stopping") return { kind: "stopping" };
        return { kind: "failed", message: e.message };
      }),
    );
    return () => {
      stop();
      void native.setCloseArmed(false);
    };
  }, [native]);

  if (!native || phase.kind === "idle") return null;
  const answer = (a: CloseAnswer, remember = false) => void native.answerClose(a, remember);

  if (phase.kind === "stopping") {
    return (
      <Modal onEscape={null}>
        <p>Stopping the daemon…</p>
      </Modal>
    );
  }
  if (phase.kind === "failed") {
    const cancel = () => {
      answer("cancel");
      setPhase({ kind: "idle" });
    };
    return (
      <Modal onEscape={cancel}>
        <h2>The daemon did not stop</h2>
        <p className="dialog-body">{phase.message}</p>
        <div className="form-actions">
          <Button onClick={() => answer("close-anyway")}>Close anyway</Button>
          <Button variant="primary" onClick={cancel}>
            Cancel
          </Button>
        </div>
      </Modal>
    );
  }
  const cancel = () => {
    answer("cancel");
    setPhase({ kind: "idle" });
  };
  return (
    <Modal onEscape={cancel}>
      <h2>Closing Fleet Manager</h2>
      <p className="dialog-body">{dialogText(summarize(agents))}</p>
      <Check checked={phase.remember} onChange={(remember) => setPhase({ kind: "asking", remember })}>
        Remember my choice
      </Check>
      <div className="form-actions">
        {/* The safe choice, focused so Enter keeps the daemon, as egui focuses it. */}
        <Button variant="primary" autoFocus onClick={() => answer("keep", phase.remember)}>
          Keep the daemon running
        </Button>
        <Button
          variant="quiet-danger"
          onClick={() => {
            answer("stop", phase.remember);
            setPhase({ kind: "stopping" });
          }}
        >
          Stop the daemon too
        </Button>
        <Button onClick={cancel}>Cancel</Button>
      </div>
      <p className="note">You can change this later in Settings.</p>
    </Modal>
  );
}

function Modal({ onEscape, children }: { onEscape: (() => void) | null; children: ReactNode }) {
  useEffect(() => {
    if (!onEscape) return;
    const key = (e: KeyboardEvent) => e.key === "Escape" && onEscape();
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [onEscape]);
  return (
    <div className="dialog-backdrop">
      <section className="dialog close-dialog" role="dialog" aria-modal="true">
        {children}
      </section>
    </div>
  );
}
