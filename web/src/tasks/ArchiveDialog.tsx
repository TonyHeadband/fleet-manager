import { useCallback, useEffect, useId, useRef, useState } from "react";
import { PLAN_CHANGED, type FleetClient } from "../api/client";
import type { ArchivePlan } from "../api/generated/ArchivePlan";
import type { ArchiveReport } from "../api/generated/ArchiveReport";
import { formatBytes } from "../api/schema";
import type { Notice } from "../shell/messages";
import { Button } from "../ui/Button";

// A port of crates/fleet-gui/src/components/archive_dialog.rs and of app.rs's archive flow.

export interface ArchiveTarget {
  kind: "task" | "orphan";
  id: string;
  /** What the footer calls it. */
  name: string;
}

/** A dialog always starts measuring: a plan is only shown for the open it was fetched for, so what is confirmed is
 * what is on disk now. */
type PlanView = { state: "measuring" } | { state: "ready"; plan: ArchivePlan } | { state: "failed"; error: string };

export interface OpenArchive extends ArchiveTarget {
  plan: PlanView;
  /** The plan request whose answer `plan` waits for; any other is stale. */
  seq: number;
  archiving: boolean;
  /** Why the plan on screen is not the first one: the daemon refused the confirmed plan as stale. */
  notice: string | null;
}

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** The footer line for a finished archive: what it freed, or the first thing it kept (the daemon's note is
 * "kept <path>: <why>", several joined by "; "). */
export function archivedText(name: string, report: ArchiveReport): string {
  if (report.kept === null) return `${name} archived, ${formatBytes(report.freed)} freed`;
  const first = report.kept.split("; ")[0]!.replace(/^kept /, "");
  const cut = first.indexOf(": ");
  return `${name} archived, kept ${cut === -1 ? first : first.slice(0, cut)}`;
}

/**
 * The open dialog and the archives in flight, kept above the Tasks view: an archive that lands after the view or the
 * dialog moved on is still announced, and closes the dialog only when it is still that target's.
 */
export function useArchive(client: FleetClient, say: (notice: Notice, text: string) => void, onArchived: () => void) {
  const [dialog, setDialog] = useState<OpenArchive | null>(null);
  const plansSent = useRef(0);
  // Read synchronously, so a second Archive… while one runs is ignored before a render catches up.
  const archiving = useRef(new Map<string, string>());
  // The open dialog's target as the last render left it, for an archive that lands after it moved on.
  const stillOpen = useRef<string | null>(null);
  stillOpen.current = dialog?.id ?? null;

  /** Always a new request: one that never answers must not leave later dialogs measuring. */
  const fetchPlan = useCallback(
    (t: ArchiveTarget) => {
      const seq = ++plansSent.current;
      setDialog((d) => (d && d.id === t.id ? { ...d, seq } : d));
      (t.kind === "orphan" ? client.orphanPlan(t.id) : client.archivePlan(t.id))
        .then(
          (plan): PlanView => ({ state: "ready", plan }),
          (e: unknown): PlanView => ({ state: "failed", error: message(e) }),
        )
        .then((plan) =>
          // Only the request the dialog waits for: an older one may answer late with what the task held back then.
          setDialog((d) => (d && d.id === t.id && d.seq === seq && d.plan.state === "measuring" ? { ...d, plan } : d)),
        );
    },
    [client],
  );

  const open = (t: ArchiveTarget) => {
    if (archiving.current.has(t.id)) return;
    setDialog({ ...t, plan: { state: "measuring" }, seq: 0, archiving: false, notice: null });
    fetchPlan(t);
  };

  const cancel = () => setDialog((d) => (d && !d.archiving ? null : d));

  const go = () => {
    // Archive is enabled only with a plan on screen: that plan is what the daemon checks against.
    if (!dialog || dialog.plan.state !== "ready" || dialog.archiving) return;
    const { plan } = dialog.plan;
    const target: ArchiveTarget = { kind: dialog.kind, id: dialog.id, name: dialog.name };
    setDialog({ ...dialog, archiving: true });
    archiving.current.set(target.id, target.name);
    (target.kind === "orphan" ? client.archiveOrphan(target.id, plan) : client.archive(target.id, plan)).then(
      (report) => landed(target, { report }),
      (e: unknown) => landed(target, { error: message(e) }),
    );
  };

  const landed = (t: ArchiveTarget, result: { report: ArchiveReport } | { error: string }) => {
    const name = archiving.current.get(t.id) ?? t.name;
    archiving.current.delete(t.id);
    // Nothing was removed: the dialog, if still this target's, shows the plan as it is now, to confirm again.
    if ("error" in result && result.error === PLAN_CHANGED && stillOpen.current === t.id) {
      setDialog((d) =>
        d && d.id === t.id
          ? { ...d, archiving: false, plan: { state: "measuring" }, notice: "This task changed since the plan was made. Review it again." }
          : d,
      );
      fetchPlan(t);
      onArchived();
      return;
    }
    setDialog((d) => (d && d.id === t.id ? null : d));
    if ("report" in result) say("success", archivedText(name, result.report));
    else say("error", `Could not archive ${name}: ${result.error}`);
    onArchived();
  };

  return { dialog, open, cancel, go };
}

interface Props {
  dialog: OpenArchive;
  onGo: () => void;
  onCancel: () => void;
}

/** The plan and Cancel / Archive. Cancel, Escape and a click outside cancel, except while the archive runs. */
export function ArchiveDialog({ dialog: d, onGo, onCancel }: Props) {
  const titleId = useId();
  const cancel = useRef<HTMLButtonElement>(null);
  // egui's `cancel_button`: Cancel takes the focus whenever nothing has it, as when Archive, focused, went disabled
  // (a browser drops the focus from a disabled button, or leaves it there where no key reaches it).
  useEffect(() => {
    const focused = document.activeElement;
    if (focused === null || focused === document.body || focused.matches(":disabled")) cancel.current?.focus();
  });
  useEffect(() => {
    const key = (e: KeyboardEvent) => e.key === "Escape" && onCancel();
    document.addEventListener("keydown", key);
    return () => document.removeEventListener("keydown", key);
  }, [onCancel]);
  return (
    <div className="dialog-backdrop" onClick={(e) => e.target === e.currentTarget && onCancel()}>
      <div className="dialog archive-dialog" role="dialog" aria-modal="true" aria-labelledby={titleId}>
        <h2 id={titleId}>{`Archive ${d.name}?`}</h2>
        {d.notice && <p className="dialog-body high">{d.notice}</p>}
        {d.plan.state === "measuring" && <p className="low">Measuring…</p>}
        {d.plan.state === "failed" && <p role="alert">{d.plan.error}</p>}
        {d.plan.state === "ready" && <PlanRows plan={d.plan.plan} />}
        <div className="form-actions">
          {/* Cancelling mid-archive would leave the passport's Archive… dead until the result lands. */}
          {/* Cancel, the safe default, takes the focus; the plan arriving above it leaves it there. */}
          <Button ref={cancel} autoFocus disabled={d.archiving} onClick={onCancel}>
            Cancel
          </Button>
          <Button variant="danger" disabled={d.plan.state !== "ready" || d.archiving} onClick={onGo}>
            Archive
          </Button>
          {d.archiving && <span className="low archiving">Archiving…</span>}
        </div>
      </div>
    </div>
  );
}

/** One row per item: what is removed with its size and any work it loses, and what stays. */
function PlanRows({ plan }: { plan: ArchivePlan }) {
  return (
    <>
      <p className="dialog-body">What archiving does:</p>
      <div className="archive-plan">
        {plan.items.map((item) => (
          <div key={item.path} className="archive-item" data-stays={item.stays || undefined}>
            <code>{item.path}</code>
            <span>{formatBytes(item.bytes)}</span>
            {item.stays ? (
              <span className="low" title="Kept on disk: archiving does not remove it">
                stays
              </span>
            ) : item.uncommitted_files > 0 ? (
              <span className="danger">{`${item.uncommitted_files} uncommitted ${item.uncommitted_files === 1 ? "file" : "files"} will be lost`}</span>
            ) : (
              <span />
            )}
          </div>
        ))}
      </div>
      <p className="dialog-body high">{`Frees ${formatBytes(plan.total)}`}</p>
      <p className="archive-note low">Branches are kept: only uncommitted work is lost.</p>
    </>
  );
}
