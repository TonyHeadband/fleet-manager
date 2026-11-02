import type { Agent } from "../api/generated/Agent";
import type { Task } from "../api/generated/Task";
import type { Tone } from "../api/generated/Tone";
import { taskOutcome } from "../tasks/model";

/** Where a task's work is, for the card's badge row: the right-hand badge, beside the agent's state on the left. */
export interface Stage {
  text: string;
  tone?: Tone;
  /** The badge's hover text: what the stage means, and the PR when there is one. */
  title: string;
  /** The one PR the stage is about, which the side panel links to. */
  url?: string;
}

/** `#499` from a GitHub PR URL; the URL itself when it is not one. */
const prNumber = (url: string) => {
  const n = url.match(/\/pull\/(\d+)/)?.[1];
  return n ? `#${n}` : url;
};

/**
 * The furthest stage the task's work has reached, its PR first: merged, a PR open (or closed with none open), the
 * last gate run, commits on the branch, changes in a worktree. None before any work. The PR comes from what the
 * merge watcher last saw on GitHub, or from the last gate run before it has looked.
 */
export function workStage(agent: Agent, task: Task | undefined): Stage | null {
  if (task) {
    const outcome = taskOutcome(task);
    if (outcome === "merged") {
      const merged = (task.prs ?? []).filter((p) => p.state === "merged");
      return { text: "Merged", tone: "done", title: "Every PR of the task is merged", ...(merged.length === 1 && { url: merged[0]!.url }) };
    }
    if (outcome === "done_by_hand") return { text: "Done by hand", tone: "done", title: "Marked done by hand" };
    const prs = task.prs ?? [];
    const lastRun = task.gate_runs?.at(-1);
    const open = prs.filter((p) => p.state === "open").map((p) => p.url);
    const opened = lastRun?.pr_url;
    if (opened && !prs.some((p) => p.url === opened)) open.push(opened);
    if (open.length === 1) return { text: `PR ${prNumber(open[0]!)}`, tone: "active", title: `PR open: ${open[0]}`, url: open[0] };
    if (open.length > 1) return { text: `${open.length} PRs open`, tone: "active", title: `PRs open: ${open.join(", ")}` };
    if (prs.some((p) => p.state === "closed")) return { text: "PR closed", tone: "caution", title: "The task's PR was closed without merging" };
    if (lastRun) {
      const failed = lastRun.steps.find((s) => s.exit_code !== 0);
      if (failed) return { text: "Gate failed", tone: "danger", title: `The gate's ${failed.name} step failed` };
    }
    if (task.repos.some((w) => w.changed)) return { text: "Committed", title: "Commits on the task's branch, no PR yet" };
  }
  if (agent.has_changes) return { text: "Changes", title: "Changes in the task's worktree, not in a PR yet" };
  return null;
}
