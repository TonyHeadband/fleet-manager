//! Applies one hook event to the store, after authenticating it with the agent's token.
use crate::status::{parse_hook, reduce, AgentState, ParseError};
use crate::store::{Store, StoreError};
use fleet_api::{AgentId, BlockedReason, Event, Status, StopCause};
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
    authenticate(store, agent, token_header).await?;
    let event = parse_hook(&body)?;
    let updated = store
        .update_agent(agent, |a| {
            let prev = AgentState {
                status: a.status,
                activity: a.activity.clone(),
                paused: a.paused,
                blocked: a.blocked_reason,
            };
            let Some(next) = reduce(&prev, &event) else {
                return;
            };
            // A pending repo request owns the card until it is decided: the hooks around the held MCP call (a
            // PreToolUse, a Stop from a background agent) say nothing about whether the owner has answered.
            // Only a real exit ends it. A proposed breakdown owns the card the same way until it is decided, except
            // against a prompt of its own (a permission, a question): that one needs the owner first, and the
            // breakdown stays pending on the record and in the side panel.
            let pinned = a.repo_request.is_some()
                || (a.blocked_reason == Some(BlockedReason::Breakdown)
                    && next.status != Status::NeedsYou);
            if pinned && next.status != Status::Stopped {
                a.activity = next.activity;
                a.paused = next.paused;
                return;
            }
            a.repo_request = None;
            if next.status != a.status {
                a.status_since = now;
            }
            a.status = next.status;
            a.activity = next.activity;
            a.paused = next.paused;
            a.blocked_reason = next.blocked;
            if next.status == Status::Stopped {
                a.stop_cause = Some(StopCause::Exited);
            }
        })
        .await?;
    Ok(if updated.is_empty() {
        IngestOutcome::Ignored
    } else {
        IngestOutcome::Applied(updated)
    })
}

/// The agent's own token, compared in constant time; shared by the hook and MCP routes.
pub async fn authenticate(
    store: &Store,
    agent: &AgentId,
    token_header: Option<&str>,
) -> Result<(), IngestError> {
    let expected = store
        .agent_token(agent)
        .await?
        .ok_or(IngestError::UnknownAgent)?;
    match token_header {
        Some(t) if !t.is_empty() && constant_time_eq(t.as_bytes(), expected.as_bytes()) => Ok(()),
        _ => Err(IngestError::BadToken),
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
