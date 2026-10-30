import { useEffect, useId, useState } from "react";
import type { FleetClient } from "../api/client";
import { missingRequired, toolName } from "../api/tools";
import type { Native } from "../native/native";
import { FolderRow } from "../startup/FirstRun";
import { RequirementsBody } from "../startup/Requirements";
import { useToolCheck, type ToolCheck } from "../startup/toolCheck";
import { Button } from "../ui/Button";
import { Modal } from "../ui/Modal";
import { pendingFolders, sameFolder, type DaemonSettingsApi } from "./daemonSettings";
import type { RestartApi } from "./restart";

// A port of egui's relaunched setup wizard (crates/fleet-gui/src/app/setup.rs, `relaunched_wizard`): Requirements,
// the folders when this window can write the daemon's config, then Done. Folders it writes wait for a restart.

type Step = "requirements" | "folders" | "done";

interface Props {
  client: FleetClient;
  native: Native | null;
  /** The daemon is on this machine: Install is offered, and each answer's PATH is adopted. */
  local: boolean;
  daemon: DaemonSettingsApi;
  restart: RestartApi;
  onClose: () => void;
}

/** egui's `done_text`: never "everything is there" without the daemon's answer saying so. */
export function doneText(check: ToolCheck): string {
  const missing = check.list.tools.filter((t) => t.required && t.error !== null).map((t) => toolName(t.tool));
  // A pending answer (a check, or an install's recheck) may change the list.
  if (check.checking || check.installing !== null) return "Still checking the tools…";
  if (check.list.tools.length === 0 || check.note !== null) return "Unable to check the tools.";
  if (!missingRequired(check.list)) return "Everything Fleet Manager needs is there.";
  return `Still missing: ${missing.join(", ")}. Check again from Settings → Extras, or relaunch this wizard.`;
}

export function RelaunchedWizard({ client, native, local, daemon, restart, onClose }: Props) {
  const check = useToolCheck(client, native, { canInstall: native !== null && local, local });
  const recheck = check.recheck;
  useEffect(recheck, [recheck]);
  const saved = daemon.saved;
  // The Folders step needs this window to write the daemon's config.
  const folders = native !== null && saved !== null;
  const [step, setStep] = useState<Step>("requirements");
  // Pre-filled with the configured folders: those the wizard wrote before, else the ones the daemon runs with.
  const [root, setRoot] = useState(saved?.tasksRoot ?? "");
  const [personas, setPersonas] = useState(saved?.personasDir ?? "");
  const [error, setError] = useState<string | null>(null);
  const titleId = useId();

  const total = folders ? 3 : 2;
  const heading = (n: number, title: string) => (
    <h2 id={titleId}>
      Step {n} of {total} · {title}
    </h2>
  );
  const pending = saved !== null && pendingFolders(saved) !== null;

  /** egui's `save_wizard_folders`: each folder that differs from the configured one, in order, without a restart. A
   * folder written before a later one fails stays written. */
  const next = async () => {
    if (!native || !saved || restart.daemonBlocker !== null) return;
    try {
      if (!sameFolder(root, saved.tasksRoot, saved.home)) await native.saveSettings({ kind: "tasksRoot", text: root });
      if (!sameFolder(personas, saved.personasDir, saved.home)) await native.saveSettings({ kind: "personasDir", text: personas });
      setError(null);
      setStep("done");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      daemon.wizardSaved();
    }
  };

  return (
    <Modal labelledBy={titleId} className="wizard relaunched-wizard" onClose={onClose}>
      {step === "requirements" && (
        <>
          {heading(1, "What Fleet Manager needs")}
          <RequirementsBody native={native} check={check} onContinue={() => setStep(folders ? "folders" : "done")} />
        </>
      )}
      {step === "folders" && native && (
        <>
          {heading(2, "Where Fleet Manager lives")}
          <p>A changed folder is saved now and used when the background service next starts.</p>
          {/* A form, so Enter in a field is Next. */}
          <form
            onSubmit={(e) => {
              e.preventDefault();
              void next();
            }}
          >
            <FolderRow caption="Tasks folder" value={root} onChange={setRoot} native={native} />
            <FolderRow caption="Personas folder" value={personas} onChange={setPersonas} native={native} />
            {error && <p className="danger">{error}</p>}
            <div className="actions">
              <Button type="submit" variant="primary" disabled={restart.daemonBlocker !== null} title={restart.daemonBlocker ?? undefined}>
                Next →
              </Button>
              <Button onClick={onClose}>Close</Button>
            </div>
          </form>
        </>
      )}
      {step === "done" && (
        <>
          {heading(total, "Done")}
          <p>{doneText(check)}</p>
          {pending && <p>The new folders are saved. They take effect when the background service next starts.</p>}
          <div className="actions">
            {pending && (
              <Button
                variant="primary"
                disabled={restart.blocker !== null}
                title={restart.blocker ?? undefined}
                onClick={() => {
                  // The usual restart, asking first when agents run.
                  onClose();
                  restart.ask({ kind: "restart" });
                }}
              >
                Restart now
              </Button>
            )}
            <Button onClick={onClose}>Done</Button>
          </div>
        </>
      )}
    </Modal>
  );
}
