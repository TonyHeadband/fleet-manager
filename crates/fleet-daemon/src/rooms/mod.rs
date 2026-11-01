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
pub mod announce;
pub mod asks;
pub mod deliver;
pub mod posts;
mod watch;

use crate::store::{RoomRow, Store, StoreError};
use fleet_api::{
    Agent, AgentId, AgentKind, MemberRef, PrState, Room, RoomKind, RoomMember, Task, TaskState,
};
use std::collections::HashSet;

pub use deliver::{repost_stale_asks, start_delivery_watcher};
pub use watch::start_room_watcher;

const UNTITLED: &str = "Untitled";

pub fn product_room_id(product: &str) -> String {
    format!("product:{product}")
}

pub fn operation_room_id(conversation: &AgentId) -> String {
    format!("operation:{}", conversation.0)
}

/// The product's room, made on first use.
pub async fn product_room(store: &Store, product: &str) -> Result<String, StoreError> {
    let id = product_room_id(product);
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
    let id = operation_room_id(conversation);
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
        out.push(view(store, r, &agents).await?);
    }
    Ok(out)
}

/// The rooms the persona's owner reads: its operation rooms, live before archived, then every product room that has
/// posts. The owner's unread count needs no member row: until they first read a room, every post in it counts.
pub async fn list_for(store: &Store, persona: &str) -> Result<Vec<Room>, StoreError> {
    let agents = store.snapshot().await?.agents;
    let (mut live, mut archived, mut products) = (Vec::new(), Vec::new(), Vec::new());
    for r in store.rooms().await? {
        let shelf = match r.kind {
            RoomKind::Operation if r.persona.as_deref() != Some(persona) => continue,
            RoomKind::Operation if r.archived_at.is_some() => &mut archived,
            RoomKind::Operation => &mut live,
            RoomKind::Product if store.room_posts(&r.id, None, 1).await?.is_empty() => continue,
            RoomKind::Product => &mut products,
        };
        shelf.push(view(store, r, &agents).await?);
    }
    live.extend(archived);
    live.extend(products);
    Ok(live)
}

async fn view(store: &Store, r: RoomRow, agents: &[Agent]) -> Result<Room, StoreError> {
    let mut members = Vec::new();
    for id in supervisors_of(store, &r, agents).await? {
        let name = member_name(store, &r, &MemberRef::Agent(id.clone())).await?;
        members.push(RoomMember { name, id });
    }
    // The owner joins by reading and the daemon only announces: neither is someone to address. Nor is a conversation
    // that only looked in.
    for id in present_agents(store, &r.id).await? {
        if !is_supervisor(agents, &id) {
            let name = member_name(store, &r, &MemberRef::Agent(id.clone())).await?;
            members.push(RoomMember { name, id });
        }
    }
    Ok(Room {
        title: title_with(store, &r, agents).await?,
        unread: store.unread_posts(&r.id, MemberRef::Owner.key()).await?,
        members,
        id: r.id,
        kind: r.kind,
        product: r.product,
        conversation: r.conversation,
        persona: r.persona,
        archived_at: r.archived_at,
    })
}

/// A member as people read it: the task's title or name, `supervisor` for a supervisor conversation, `owner`, or
/// `Fleet Manager`.
pub(crate) async fn member_name(
    store: &Store,
    room: &RoomRow,
    m: &MemberRef,
) -> Result<String, StoreError> {
    Ok(match m {
        MemberRef::Owner => "owner".into(),
        MemberRef::Daemon => "Fleet Manager".into(),
        MemberRef::Agent(a) if room.conversation.as_ref() == Some(a) => SUPERVISOR.into(),
        MemberRef::Agent(a) => match store.task(a).await? {
            Some(t) => t.title.filter(|t| !t.is_empty()).unwrap_or(t.name),
            None if is_supervisor(&store.snapshot().await?.agents, a) => SUPERVISOR.into(),
            None => a.0.clone(),
        },
    })
}

const SUPERVISOR: &str = "supervisor";

/// The supervisor conversations the room wakes and lists: an operation room's own conversation; in a product room,
/// for each persona one of whose conversations looked in, that persona's most recently used conversation (as
/// `to: "supervisor"` resolves it). Other conversations that looked in only read: their reads are tracked, but
/// nothing wakes them.
pub(crate) async fn supervisors_of(
    store: &Store,
    room: &RoomRow,
    agents: &[Agent],
) -> Result<Vec<AgentId>, StoreError> {
    if room.kind == RoomKind::Operation {
        return Ok(room.conversation.iter().cloned().collect());
    }
    let mut personas = Vec::new();
    for id in present_agents(store, &room.id).await? {
        if let Some(a) = agents
            .iter()
            .find(|a| a.id == id && a.kind == AgentKind::Supervisor)
        {
            if !personas.contains(&a.persona) {
                personas.push(a.persona.clone());
            }
        }
    }
    let mut out = Vec::new();
    for p in personas {
        out.extend(crate::supervisor::latest_conversation(store, &p).await?);
    }
    Ok(out)
}

/// The agents in the room now, tasks and supervisor conversations alike.
pub(crate) async fn present_agents(store: &Store, room: &str) -> Result<Vec<AgentId>, StoreError> {
    Ok(store
        .room_members(room)
        .await?
        .into_iter()
        .filter(|m| m.left_at.is_none())
        .filter_map(|m| match MemberRef::from_key(&m.member) {
            MemberRef::Agent(a) => Some(a),
            _ => None,
        })
        .collect())
}

pub(crate) fn is_supervisor(agents: &[Agent], id: &AgentId) -> bool {
    agents
        .iter()
        .any(|a| &a.id == id && a.kind == AgentKind::Supervisor)
}

/// The room as people read it: its product's name, or its conversation's title.
pub async fn title(store: &Store, r: &RoomRow) -> Result<String, StoreError> {
    title_with(store, r, &store.snapshot().await?.agents).await
}

async fn title_with(store: &Store, r: &RoomRow, agents: &[Agent]) -> Result<String, StoreError> {
    Ok(match (&r.kind, &r.product, &r.conversation) {
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
    })
}

/// Whether the task is over: archived, done, or its PRs merged or closed. A PR still open keeps it; so does a changed
/// repo with no PR. A task whose branch changed nothing is over once its one repo has a PR and none is open.
pub(crate) fn ended(t: &Task) -> bool {
    if t.state == TaskState::Archived || t.done_by_hand || t.done_at.is_some() {
        return true;
    }
    if t.prs.iter().any(|p| p.state == PrState::Open) {
        return false;
    }
    let mut changed = t.repos.iter().filter(|w| w.changed).peekable();
    if changed.peek().is_none() {
        return t.repos.len() == 1 && !t.prs.is_empty();
    }
    changed.all(|w| t.prs.iter().any(|p| p.repo == w.repo))
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
    // A task whose record is gone (a start that was undone) must not hold a room open. Supervisor conversations
    // stay: a supervisor joins the rooms of its persona it looks in.
    let rooms = store.rooms().await?;
    let mut conversations: HashSet<_> = rooms
        .iter()
        .filter_map(|r| r.conversation.clone())
        .collect();
    conversations.extend(supervisors(store).await?);
    for r in &rooms {
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
    // Read first: the watcher applies every task on each pass, and a task already in an open room needs no write.
    let me = MemberRef::Agent(t.id.clone());
    let mine = store.rooms_of(me.key()).await?;
    if !t.product.is_empty() && !mine.contains(&product_room_id(&t.product)) {
        let room = product_room(store, &t.product).await?;
        join(store, &room, &me, at).await?;
    }
    if let Some(conversation) = &t.origin {
        let id = operation_room_id(conversation);
        let open = mine.contains(&id)
            && store
                .room(&id)
                .await?
                .is_some_and(|r| r.archived_at.is_none());
        if !open {
            let room = operation_room(store, conversation, &t.persona).await?;
            join(store, &room, &me, at).await?;
        }
    }
    Ok(())
}

async fn retire(store: &Store, at: i64, id: &AgentId) -> Result<(), StoreError> {
    for room in store.rooms_of(&id.0).await? {
        leave(store, &room, &MemberRef::Agent(id.clone()), at).await?;
    }
    Ok(())
}

/// Every supervisor conversation: members that are not tasks.
async fn supervisors(store: &Store) -> Result<HashSet<AgentId>, StoreError> {
    Ok(store
        .snapshot()
        .await?
        .agents
        .into_iter()
        .filter(|a| a.kind == AgentKind::Supervisor)
        .map(|a| a.id)
        .collect())
}

/// Archives each open operation room that had tasks and has none left; supervisor conversations, the owner and
/// the daemon do not hold it open.
async fn archive_emptied(store: &Store, at: i64) -> Result<(), StoreError> {
    let conversations = supervisors(store).await?;
    for r in store.rooms().await? {
        if r.kind != RoomKind::Operation || r.archived_at.is_some() {
            continue;
        }
        let members = store.room_members(&r.id).await?;
        let tasks = || {
            members.iter().filter(|m| {
                matches!(MemberRef::from_key(&m.member), MemberRef::Agent(a)
                    if Some(&a) != r.conversation.as_ref() && !conversations.contains(&a))
            })
        };
        if tasks().next().is_some() && tasks().all(|m| m.left_at.is_some()) {
            store.set_room_archived(&r.id, Some(at)).await?;
        }
    }
    Ok(())
}
