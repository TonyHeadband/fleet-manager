import type { Agent } from "../api/generated/Agent";

// fleet-client's `close::summarize` and `close::dialog_text`: the close dialog's live numbers and its words. The
// decision itself (`close_action`) is the app's, in Rust.
export interface Summary {
  running: number;
  working: number;
  needs_you: number;
}

export function summarize(agents: Agent[]): Summary {
  const count = (s: Agent["status"]) => agents.filter((a) => a.status === s).length;
  return { running: agents.length - count("stopped"), working: count("working"), needs_you: count("needs_you") };
}

export function dialogText(s: Summary): string {
  if (s.running === 0) return "No agents are running, so stopping the daemon stops nothing. Cards, branches and worktrees are kept.";
  const lead =
    s.running === 1
      ? "1 agent is running. It will stop now and resume automatically next time the daemon starts."
      : `${s.running} agents are running. They will stop now and resume automatically next time the daemon starts.`;
  const parts = [s.working > 0 && `${s.working} working`, s.needs_you > 0 && `${s.needs_you} waiting for you`].filter(Boolean);
  const detail = parts.length === 0 ? "" : ` Now: ${parts.join(", ")}.`;
  return `${lead}${detail} Cards, branches and worktrees are kept.`;
}
