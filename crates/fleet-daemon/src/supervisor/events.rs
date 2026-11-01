//! Task events: what the daemon already sees about a task, stored for the supervisor conversation that started it.
use crate::agents::AgentService;
use crate::store::{Store, StoreError};
use fleet_api::{task_id, AgentId, AgentKind, EventKind, Status, TaskEvent, TaskEventKind};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::warn;

/// Stores the event and broadcasts it. `task` may be a specialist: the event goes to its task. It goes to the
/// conversation that started the task; if that is gone or the task has no origin, to the persona's most recently
/// used one that still has its agent; with none, nowhere, but it is still stored and broadcast so it toasts.
pub async fn route(
    svc: &AgentService,
    task: &AgentId,
    kind: TaskEventKind,
    text: String,
) -> Result<(), StoreError> {
    route_with(&svc.store, svc.options().clock.now(), task, kind, text).await
}

/// `route` for a caller that holds the store and the time, not the service (the merge checker).
pub(crate) async fn route_with(
    store: &Store,
    at: i64,
    task: &AgentId,
    kind: TaskEventKind,
    text: String,
) -> Result<(), StoreError> {
    let Some(t) = store.task(task).await? else {
        return Ok(());
    };
    let conversations = store.conversations(&t.persona).await?;
    let origin = t
        .origin
        .filter(|o| conversations.iter().any(|c| &c.id == o));
    let conversation = match origin {
        Some(o) => Some(o),
        // A row whose agent is gone (a start that failed before its row could go) cannot read the event.
        None => {
            let agents = store.snapshot().await?.agents;
            conversations
                .into_iter()
                .find(|c| agents.iter().any(|a| a.id == c.id))
                .map(|c| c.id)
        }
    };
    store
        .add_task_event(&TaskEvent {
            id: 0,
            conversation,
            persona: t.persona,
            task_title: t.title.unwrap_or_else(|| t.id.0.clone()),
            task: t.id,
            kind,
            text,
            at,
        })
        .await?;
    Ok(())
}

/// Turns an agent's move into `NeedsYou` into an event. Once per transition: a repeat of the status, or a new
/// activity while it stays, is not a new event.
pub fn start_event_watcher(svc: Arc<AgentService>) -> JoinHandle<()> {
    let mut events = svc.store.subscribe();
    tokio::spawn(async move {
        let mut seen: HashMap<AgentId, Status> = HashMap::new();
        loop {
            let ev = match events.recv().await {
                Ok(ev) => ev,
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    seen.clear();
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            };
            let (id, status, activity) = match ev.kind {
                EventKind::StatusChanged {
                    id,
                    status,
                    activity,
                    ..
                } => (id, status, activity),
                EventKind::AgentRemoved { id } => {
                    seen.remove(&id);
                    continue;
                }
                _ => continue,
            };
            if seen.insert(id.clone(), status) == Some(status) || status != Status::NeedsYou {
                continue;
            }
            let agent = match svc.store.snapshot().await {
                Ok(snap) => snap.agents.into_iter().find(|a| a.id == id),
                Err(e) => {
                    warn!(error = %e, "cannot read the agent that needs you");
                    continue;
                }
            };
            let Some(a) = agent.filter(|a| a.kind != AgentKind::Supervisor) else {
                continue;
            };
            let text = format!("needs you · {activity}");
            if let Err(e) = route(&svc, task_id(&a), TaskEventKind::NeedsYou, text).await {
                warn!(error = %e, "cannot store the task's event");
            }
        }
    })
}
