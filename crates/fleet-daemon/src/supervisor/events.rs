//! Task events: what the daemon already sees about a task. The notices go to the task's product room; the event is
//! stored and broadcast so the web toasts it. The supervisor's chat does not show them.
use crate::agents::AgentService;
use crate::rooms::announce;
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
        None => latest_conversation(store, &t.persona).await?,
    };
    let task_title = t.title.unwrap_or_else(|| t.id.0.clone());
    // The task's own notices are for its product's room. Room messages and asks are posts there already, and a
    // merge is announced there as "landed" by the merge checker.
    let in_room = !matches!(
        kind,
        TaskEventKind::RoomMessage | TaskEventKind::RoomAsk | TaskEventKind::PrMerged
    );
    if !t.product.is_empty() && in_room {
        if let Err(e) = announce::task_notice(store, &t.product, &task_title, &text).await {
            warn!(task = %t.id.0, error = %e, "cannot announce the task's notice in the product room");
        }
    }
    store
        .add_task_event(&TaskEvent {
            id: 0,
            conversation,
            persona: t.persona,
            task_title,
            task: t.id,
            kind,
            text,
            at,
        })
        .await?;
    Ok(())
}

/// The persona's most recently used conversation that still has its agent: where what is for the persona's
/// supervisor, and for no conversation in particular, goes. A row whose agent is gone (a start that failed before
/// its row could go) cannot read it.
pub(crate) async fn latest_conversation(
    store: &Store,
    persona: &str,
) -> Result<Option<AgentId>, StoreError> {
    let agents = store.snapshot().await?.agents;
    Ok(store
        .conversations(persona)
        .await?
        .into_iter()
        .find(|c| agents.iter().any(|a| a.id == c.id))
        .map(|c| c.id))
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
