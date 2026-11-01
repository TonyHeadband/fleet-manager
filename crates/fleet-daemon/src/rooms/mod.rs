//! Rooms: a product's room for its orchestrators, and a room for each supervisor conversation's tasks.
//!
//! Membership follows the tasks. An orchestrator task joins its product's room when it starts, and its origin
//! conversation's operation room if a conversation started it. It leaves them when it ends, and an operation room
//! left with no task is archived until that conversation starts another.
//!
//! Starts are not read off the agent-created event: a task's origin is stored after the spawn returns, so the
//! start would see none. `record_orchestrator` calls `sync_task` for the record, and `start_task` again once it
//! has set the origin. Ends change no event of their own (the merge checker, a stop and an archive store the
//! record silently), so the watcher re-reads every task on the events that come with an end.
pub mod asks;
pub mod posts;
mod watch;

use crate::store::{RoomRow, Store, StoreError};
use fleet_api::{AgentId, AgentKind, MemberRef, PrState, Room, RoomKind, Task, TaskState};
use std::collections::HashSet;

pub use watch::start_room_watcher;

const UNTITLED: &str = "Untitled";

/// The product's room, made on first use.
pub async fn product_room(store: &Store, product: &str) -> Result<String, StoreError> {
    let id = format!("product:{product}");
    store
        .insert_room(&RoomRow {
            id: id.clone(),
            kind: RoomKind::Product,
            product: Some(product.to_string()),
            conversation: None,
            persona: None,
            created_at: crate::store::now_secs(),
            archived_at: None,
        })
        .await?;
    Ok(id)
}

/// The conversation's room, made on first use and reopened if it was archived. The conversation is a member.
pub async fn operation_room(
    store: &Store,
    conversation: &AgentId,
    persona: &str,
) -> Result<String, StoreError> {
    let id = format!("operation:{}", conversation.0);
    store
        .insert_room(&RoomRow {
            id: id.clone(),
            kind: RoomKind::Operation,
            product: None,
            conversation: Some(conversation.clone()),
            persona: Some(persona.to_string()),
            created_at: crate::store::now_secs(),
            archived_at: None,
        })
        .await?;
    store.set_room_archived(&id, None).await?;
    store
        .add_member(
            &id,
            MemberRef::Agent(conversation.clone()).key(),
            crate::store::now_secs(),
        )
        .await?;
    Ok(id)
}

/// Puts `member` in the room (no-op if they are). An agent joining an archived operation room reopens it.
pub async fn join(
    store: &Store,
    room: &str,
    member: &MemberRef,
    at: i64,
) -> Result<(), StoreError> {
    store.add_member(room, member.key(), at).await?;
    if matches!(member, MemberRef::Agent(_)) {
        if let Some(r) = store.room(room).await? {
            if r.archived_at.is_some() && r.conversation.as_ref().is_some_and(|c| !is(member, c)) {
                store.set_room_archived(room, None).await?;
            }
        }
    }
    Ok(())
}

pub async fn leave(
    store: &Store,
    room: &str,
    member: &MemberRef,
    at: i64,
) -> Result<(), StoreError> {
    store.remove_member(room, member.key(), at).await
}

fn is(member: &MemberRef, agent: &AgentId) -> bool {
    matches!(member, MemberRef::Agent(a) if a == agent)
}

/// Every room, with the title and the owner's unread count.
pub async fn list(store: &Store) -> Result<Vec<Room>, StoreError> {
    let agents = store.snapshot().await?.agents;
    let mut out = Vec::new();
    for r in store.rooms().await? {
        let title = match (&r.kind, &r.product, &r.conversation) {
            (RoomKind::Product, Some(p), _) => match store.product(p).await? {
                Some(row) => row.name,
                None => p.clone(),
            },
            (_, _, Some(c)) => agents
                .iter()
                .find(|a| &a.id == c)
                .and_then(|a| a.title.clone())
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| UNTITLED.to_string()),
            _ => UNTITLED.to_string(),
        };
        out.push(Room {
            unread: store.unread_posts(&r.id, MemberRef::Owner.key()).await?,
            id: r.id,
            kind: r.kind,
            title,
            product: r.product,
            conversation: r.conversation,
            persona: r.persona,
            archived_at: r.archived_at,
        });
    }
    Ok(out)
}

/// Whether the task is over: archived, done, or every one of its PRs merged or closed. A task at Review with an
/// open PR, or with none yet, is not.
fn ended(t: &Task) -> bool {
    t.state == TaskState::Archived
        || t.done_at.is_some()
        || t.done_by_hand
        || (!t.prs.is_empty() && t.prs.iter().all(|p| p.state != PrState::Open))
}

/// Brings the one task's rooms in line with the record: joined while it runs, left once it has ended.
pub async fn sync_task(store: &Store, at: i64, id: &AgentId) -> Result<(), StoreError> {
    match store.task(id).await? {
        Some(t) if t.kind == AgentKind::Orchestrator => apply(store, at, &t).await?,
        _ => retire(store, at, id).await?,
    }
    archive_emptied(store, at).await
}

/// `sync_task` for every task, and the archiving of rooms no task is left in.
pub async fn sync(store: &Store, at: i64) -> Result<(), StoreError> {
    let tasks = store.tasks().await?;
    let mut live = HashSet::new();
    for t in tasks.iter().filter(|t| t.kind == AgentKind::Orchestrator) {
        live.insert(t.id.clone());
        apply(store, at, t).await?;
    }
    // A task whose record is gone (a start that was undone) must not hold a room open.
    let conversations: HashSet<_> = store
        .rooms()
        .await?
        .into_iter()
        .filter_map(|r| r.conversation)
        .collect();
    for r in store.rooms().await? {
        for m in store.room_members(&r.id).await? {
            if m.left_at.is_none() {
                if let MemberRef::Agent(a) = MemberRef::from_key(&m.member) {
                    if !live.contains(&a) && !conversations.contains(&a) {
                        store.remove_member(&r.id, &m.member, at).await?;
                    }
                }
            }
        }
    }
    archive_emptied(store, at).await
}

async fn apply(store: &Store, at: i64, t: &Task) -> Result<(), StoreError> {
    if ended(t) {
        return retire(store, at, &t.id).await;
    }
    let me = MemberRef::Agent(t.id.clone());
    if !t.product.is_empty() {
        let room = product_room(store, &t.product).await?;
        join(store, &room, &me, at).await?;
    }
    if let Some(conversation) = &t.origin {
        let room = operation_room(store, conversation, &t.persona).await?;
        join(store, &room, &me, at).await?;
    }
    Ok(())
}

async fn retire(store: &Store, at: i64, id: &AgentId) -> Result<(), StoreError> {
    for room in store.rooms_of(&id.0).await? {
        leave(store, &room, &MemberRef::Agent(id.clone()), at).await?;
    }
    Ok(())
}

/// Archives each open operation room that had tasks and has none left; its conversation, the owner and the
/// daemon do not hold it open.
async fn archive_emptied(store: &Store, at: i64) -> Result<(), StoreError> {
    for r in store.rooms().await? {
        if r.kind != RoomKind::Operation || r.archived_at.is_some() {
            continue;
        }
        let members = store.room_members(&r.id).await?;
        let tasks = || {
            members.iter().filter(|m| {
                matches!(MemberRef::from_key(&m.member), MemberRef::Agent(a) if Some(&a) != r.conversation.as_ref())
            })
        };
        if tasks().next().is_some() && tasks().all(|m| m.left_at.is_some()) {
            store.set_room_archived(&r.id, Some(at)).await?;
        }
    }
    Ok(())
}
