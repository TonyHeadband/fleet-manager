import type { Event } from "./generated/Event";
import type { Snapshot } from "./generated/Snapshot";

/** Applies one event to the board. Events at or below `state.seq` are replays after a reconnect. */
export function boardReducer(state: Snapshot, e: Event): Snapshot {
  if (e.seq <= state.seq) return state;
  const k = e.kind;
  switch (k.type) {
    case "agent_created":
      return { seq: e.seq, agents: state.agents.some((a) => a.id === k.agent.id) ? state.agents : [...state.agents, k.agent] };
    case "status_changed":
      return {
        seq: e.seq,
        agents: state.agents.map((a) =>
          a.id === k.id
            ? { ...a, status: k.status, activity: k.activity, status_since: k.since, has_changes: k.has_changes, paused: k.paused, blocked_reason: k.blocked_reason }
            : a,
        ),
      };
    case "workspaces_changed":
      return {
        seq: e.seq,
        agents: state.agents.map((a) =>
          a.id === k.id ? { ...a, workspaces: k.workspaces, has_changes: k.workspaces.some((w) => w.has_changes) } : a,
        ),
      };
    case "title_changed":
      return { seq: e.seq, agents: state.agents.map((a) => (a.id === k.id ? { ...a, title: k.title } : a)) };
    case "agent_removed":
      return { seq: e.seq, agents: state.agents.filter((a) => a.id !== k.id) };
  }
}
