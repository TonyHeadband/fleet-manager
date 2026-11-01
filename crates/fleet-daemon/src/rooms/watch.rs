//! The room watcher: keeps room membership in step with the tasks as they end.
use super::sync;
use crate::agents::AgentService;
use fleet_api::EventKind;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::warn;

/// Syncs once at start, then on each event that comes with a task's end: a status change (a stop, Review), an
/// agent removed (archive), and a task event (the merge checker's PR merged). Missing an event only delays a
/// leave until the next one, so `Lagged` just syncs.
pub fn start_room_watcher(svc: Arc<AgentService>) -> JoinHandle<()> {
    let mut events = svc.store.subscribe();
    tokio::spawn(async move {
        loop {
            if let Err(e) = sync(&svc.store, svc.options().clock.now()).await {
                warn!(error = %e, "cannot sync the rooms");
            }
            loop {
                match events.recv().await {
                    Ok(ev) => {
                        if matches!(
                            ev.kind,
                            EventKind::StatusChanged { .. }
                                | EventKind::AgentRemoved { .. }
                                | EventKind::SupervisorEvent { .. }
                        ) {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => break,
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        }
    })
}
