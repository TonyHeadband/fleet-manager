//! Applies the daemon's events to a board snapshot. Port of `web/src/api/board.ts`.
use fleet_api::{Event, EventKind, Snapshot};

/// Events at or below `state.seq` are replays after a reconnect and change nothing.
pub fn apply(state: &mut Snapshot, e: &Event) {
    if e.seq <= state.seq {
        return;
    }
    state.seq = e.seq;
    match &e.kind {
        EventKind::AgentCreated { agent } => {
            if !state.agents.iter().any(|a| a.id == agent.id) {
                state.agents.push(agent.clone());
            }
        }
        EventKind::StatusChanged { id, status, activity, since, has_changes, paused, blocked_reason, repo_request } => {
            if let Some(a) = state.agents.iter_mut().find(|a| &a.id == id) {
                a.status = *status;
                a.activity = activity.clone();
                a.status_since = *since;
                a.has_changes = *has_changes;
                a.paused = *paused;
                a.blocked_reason = *blocked_reason;
                a.repo_request = repo_request.clone();
            }
        }
        EventKind::WorkspacesChanged { id, workspaces } => {
            if let Some(a) = state.agents.iter_mut().find(|a| &a.id == id) {
                a.has_changes = workspaces.iter().any(|w| w.has_changes);
                a.workspaces = workspaces.clone();
            }
        }
        EventKind::AgentRemoved { id } => state.agents.retain(|a| &a.id != id),
    }
}
