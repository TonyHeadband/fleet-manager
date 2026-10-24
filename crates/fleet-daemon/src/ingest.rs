//! Applies one hook event to the store, after authenticating it with the agent's token.
use crate::status::{parse_hook, reduce, AgentState, ParseError};
use crate::store::{Store, StoreError};
use fleet_api::{AgentId, Event};
use serde_json::Value;

#[derive(Debug)]
pub enum IngestOutcome {
    Applied(Vec<Event>),
    /// Valid event that changes nothing: duplicate, unknown type, or the agent already ended.
    Ignored,
}

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("unknown agent")]
    UnknownAgent,
    #[error("bad or missing token")]
    BadToken,
    #[error("bad hook body: {0}")]
    BadBody(#[from] ParseError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// `now` is unix seconds, passed in so status_since is deterministic under test.
pub async fn ingest(
    store: &Store,
    agent: &AgentId,
    token_header: Option<&str>,
    body: Value,
    now: i64,
) -> Result<IngestOutcome, IngestError> {
    let expected = store.agent_token(agent).await?.ok_or(IngestError::UnknownAgent)?;
    match token_header {
        Some(t) if !t.is_empty() && constant_time_eq(t.as_bytes(), expected.as_bytes()) => {}
        _ => return Err(IngestError::BadToken),
    }
    let event = parse_hook(&body)?;
    let updated = store
        .update_agent(agent, |a| {
            let prev = AgentState {
                status: a.status,
                activity: a.activity.clone(),
                paused: a.paused,
                blocked: a.blocked_reason,
            };
            if let Some(next) = reduce(&prev, &event) {
                if next.status != a.status {
                    a.status_since = now;
                }
                a.status = next.status;
                a.activity = next.activity;
                a.paused = next.paused;
                a.blocked_reason = next.blocked;
            }
        })
        .await?;
    Ok(if updated.is_empty() { IngestOutcome::Ignored } else { IngestOutcome::Applied(updated) })
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
