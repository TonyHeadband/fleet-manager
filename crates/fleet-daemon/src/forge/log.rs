//! Every forge call goes through `recorded`, which leaves one `forge_calls` row behind.
use super::{ForgeError, RepoRef};
use crate::store::{ForgeCallRow, Store};
use fleet_api::AgentId;
use std::time::Instant;
use tracing::warn;

/// Who a forge call is made for.
#[derive(Clone, Debug)]
pub struct CallCtx {
    pub persona: String,
    pub task: Option<AgentId>,
    pub agent: Option<AgentId>,
}

/// Runs the blocking `f` off the async runtime and records it. A failed insert is logged, never the call's error.
pub async fn recorded<T: Send + 'static>(
    store: &Store,
    ctx: &CallCtx,
    forge: &'static str,
    repo: &RepoRef,
    op: &'static str,
    f: impl FnOnce() -> Result<T, ForgeError> + Send + 'static,
) -> Result<T, ForgeError> {
    let started = Instant::now();
    let result = tokio::task::spawn_blocking(f)
        .await
        .unwrap_or_else(|e| Err(ForgeError::Transport(format!("the call panicked: {e}"))));
    let row = ForgeCallRow {
        id: 0,
        at: 0,
        persona: ctx.persona.clone(),
        task: ctx.task.clone(),
        agent: ctx.agent.clone(),
        forge: forge.into(),
        repo: repo.slug(),
        op: op.into(),
        ok: result.is_ok(),
        detail: result.as_ref().err().map(|e| e.to_string()),
        ms: started.elapsed().as_millis() as i64,
    };
    if let Err(e) = store.record_forge_call(&row).await {
        warn!("could not record the {op} call on {}: {e}", row.repo);
    }
    result
}
