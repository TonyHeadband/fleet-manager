import type { Workspace } from "../api/generated/Workspace";

/** Lite's `merge_note`: how a task branch started from a local branch goes back into it. */
function mergeNote(branch: string, from: string): string {
  return `Merge ${branch} into ${from} first (a fast-forward if you did not commit there meanwhile), then merge ${from} as usual.`;
}

/** Lite's `workspace_tip`: where the task branch started and, when that was a local branch, how to merge it back. */
export function workspaceTip(w: Workspace): string {
  if (w.owned) return `${w.branch} · created in this task`;
  const start = w.base.slice(0, 7);
  const from = w.from_branch;
  // Nobody merges into a remote-tracking ref, so a URL repo's origin/<branch> start has no route to explain.
  if (from?.startsWith("origin/")) return `${w.branch}, started from ${from} at ${start}`;
  if (from) return `${w.branch}, started from ${from} at ${start}\n\n${mergeNote(w.branch, from)}`;
  return `${w.branch}, started at ${start}`;
}
