import { useEffect, useRef, type ReactNode, useState } from "react";
import type { CiState } from "../api/generated/CiState";
import type { MergedPr } from "../api/generated/MergedPr";
import type { Task } from "../api/generated/Task";
import type { Tone } from "../api/generated/Tone";
import { ConfirmDialog } from "../ui/ConfirmDialog";

/** The two daemon calls the button makes. */
export interface MergeApi {
  refresh: (task: string) => Promise<Task>;
  merge: (task: string, repo: string) => Promise<MergedPr>;
}

const TONE: Record<CiState, Tone> = { green: "done", red: "danger", pending: "muted", unknown: "muted" };

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

const prNumber = (url: string) => url.match(/\/pull\/(\d+)/)?.[1] ?? url;

/**
 * The details row's Merge button, a stage badge that acts: green and enabled when the open PR's CI is green and
 * GitHub says it can merge, red and disabled when CI failed (its title lists the failing checks), grey and disabled
 * while the checks run or cannot be read. Opening the task reads its PRs from GitHub once. A click asks first.
 * The daemon refuses a merge that is not allowed; its text is shown as it is.
 */
export function useMerge(task: Task | undefined, api: MergeApi, onChanged: () => void): { button: ReactNode; outcome: ReactNode } {
  const [asking, setAsking] = useState(false);
  const [busy, setBusy] = useState(false);
  const [outcome, setOutcome] = useState<{ ok: boolean; text: string } | null>(null);
  const id = task?.id;
  const pr = task?.prs?.find((p) => p.state === "open");
  const hasOpenPr = pr !== undefined;

  // The task's state is read from GitHub when the panel opens it, not on every change the refresh itself causes.
  const refreshed = useRef<string | null>(null);
  const latest = useRef({ api, onChanged });
  latest.current = { api, onChanged };
  useEffect(() => {
    setOutcome(null);
    setAsking(false);
  }, [id]);
  useEffect(() => {
    if (!id || !hasOpenPr || refreshed.current === id) return;
    refreshed.current = id;
    latest.current.api.refresh(id).then(
      () => latest.current.onChanged(),
      // The stored state stays on show; a failed read is not worth a message of its own.
      () => {},
    );
  }, [id, hasOpenPr]);

  if (!task || !pr) return { button: null, outcome: null };
  const ready = pr.ci === "green" && pr.can_merge;
  const tone = TONE[pr.ci];
  const title =
    pr.ci === "red"
      ? `CI failed: ${pr.failing_checks.length ? pr.failing_checks.join(", ") : "a check failed"}`
      : pr.ci === "pending"
        ? "Checks are still running"
        : pr.ci === "unknown"
          ? "The checks cannot be read"
          : pr.can_merge
            ? "CI is green: merge the PR"
            : "CI is green, but GitHub says the PR cannot merge yet";
  const number = prNumber(pr.url);
  // The branch the PR lands on is the task's base for that repo; the daemon names the method only once it has merged.
  const base = task.repos.find((w) => w.repo === pr.repo)?.from_branch ?? "its base branch";

  const merge = () => {
    setAsking(false);
    setBusy(true);
    setOutcome(null);
    api.merge(task.id, pr.repo).then(
      (m) => {
        setBusy(false);
        setOutcome({ ok: true, text: `Merged PR #${m.number} with ${m.method}.` });
        onChanged();
      },
      (e: unknown) => {
        setBusy(false);
        setOutcome({ ok: false, text: message(e) });
      },
    );
  };

  const button = (
    <>
      <button type="button" className="badge merge-button" data-field="merge" data-tone={tone} title={title} disabled={!ready || busy} onClick={() => setAsking(true)}>
        Merge
      </button>
      {asking && (
        <ConfirmDialog
          title="Merge PR"
          body={`Merge PR #${number} into ${base}?`}
          go="Merge PR"
          goVariant="primary"
          onGo={merge}
          onCancel={() => setAsking(false)}
        />
      )}
    </>
  );
  const note = outcome && (
    <p className="merge-outcome" role={outcome.ok ? "status" : "alert"} data-ok={outcome.ok}>
      {outcome.text}
    </p>
  );
  return { button, outcome: note };
}
