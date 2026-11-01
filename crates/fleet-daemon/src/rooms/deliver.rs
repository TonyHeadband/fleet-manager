//! Waking a post's recipients, and putting what is for the owner in front of them.
//!
//! A notice is never typed into a running turn, and a held tool call counts as one. An orchestrator hears of a post
//! from `wait_for_fleet` (its task's inbox) and from one line typed when it is at its input box; a supervisor
//! conversation from a message, which resumes it if it has exited, though only its operation room does that: a
//! product room post for an exited one is a chat row in it instead. An agent that cannot take the line now keeps it
//! in its pending queue until it next reaches Review, when everything queued is typed as one line.
//!
//! The queue is in memory. A daemon restart ends every session, so a queued line has nobody to be typed into; what it
//! said is not lost: the post stays in the room for `room_read`, an orchestrator's inbox is stored with its task, and
//! an answer given after the restart is delivered afresh.
use super::posts::kind_word;
use crate::agents::{one_line, AgentService, ServiceError};
use crate::store::{RoomRow, StoreError};
use crate::supervisor::{route_with, Supervisor};
use fleet_api::{
    Agent, AgentId, AgentKind, EventKind, FleetEvent, FleetEventKind, MemberRef, Post, PostKind,
    RoomKind, Status, TaskEventKind,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::warn;

/// How a notice starts, so the agent knows the line is the daemon's and not the owner's.
const FROM: &str = "Fleet Manager:";
/// How long an ask stays open before it is put to the owner again.
pub const REPOST_AFTER: i64 = 30 * 60;

#[derive(Default)]
pub struct Notices {
    /// Held from reading an agent's status to typing its queue, by `flush` alone: a line queued just as its agent
    /// goes idle is typed by the queueing side or by the watcher, never by both and never by neither.
    lock: tokio::sync::Mutex<()>,
    queue: Mutex<HashMap<AgentId, Vec<String>>>,
    /// Agents just typed into whose turn has not shown yet, with when: the status stays Review until Claude's hook
    /// reports Working, and a second line typed meanwhile would land in the turn the first one started. Set and
    /// cleared under `lock`; cleared by the next status other than Review (a turn, a stop, an exit), and ignored once
    /// the agent's status changed after it was set, which a lagged watcher may not have seen.
    awaiting: Mutex<HashMap<AgentId, i64>>,
}

impl Notices {
    pub(crate) fn push(&self, agent: &AgentId, line: String) {
        let mut q = self.queue.lock().expect("notices poisoned");
        let lines = q.entry(agent.clone()).or_default();
        // Two posts of one kind from one sender in one room read the same: once is enough.
        if !lines.contains(&line) {
            lines.push(line);
        }
    }

    fn take(&self, agent: &AgentId) -> Vec<String> {
        self.queue
            .lock()
            .expect("notices poisoned")
            .remove(agent)
            .unwrap_or_default()
    }

    /// Puts back lines that could not be typed, ahead of any queued since.
    fn restore(&self, agent: &AgentId, mut lines: Vec<String>) {
        let mut q = self.queue.lock().expect("notices poisoned");
        for l in q.remove(agent).unwrap_or_default() {
            if !lines.contains(&l) {
                lines.push(l);
            }
        }
        q.insert(agent.clone(), lines);
    }

    /// Whether the agent's turn is still to show. A status that changed in a later second than the mark means the
    /// turn came and went; one in the same second is the Review the line was typed into, so it still holds.
    fn awaiting(&self, a: &Agent) -> bool {
        let mut marks = self.awaiting.lock().expect("notices poisoned");
        match marks.get(&a.id) {
            Some(&at) if a.status_since > at => {
                marks.remove(&a.id);
                false
            }
            Some(_) => true,
            None => false,
        }
    }

    fn set_awaiting(&self, agent: &AgentId, at: Option<i64>) {
        let mut marks = self.awaiting.lock().expect("notices poisoned");
        match at {
            Some(at) => marks.insert(agent.clone(), at),
            None => marks.remove(agent),
        };
    }

    fn waiting(&self) -> Vec<AgentId> {
        self.queue
            .lock()
            .expect("notices poisoned")
            .keys()
            .cloned()
            .collect()
    }
}

/// Wakes the recipients of a message or an ask, and surfaces it to the owner when it is theirs: one addressed to
/// the owner, or an ask to the supervisor. Announcements and answers wake nobody here, nor does a breakdown's ask:
/// only the owner decides it, and they hear of it from the breakdown's own Needs you.
pub async fn deliver(svc: &Arc<AgentService>, post: &Post) -> Result<(), ServiceError> {
    if !matches!(post.kind, PostKind::Message | PostKind::Ask) || post.breakdown_version.is_some() {
        return Ok(());
    }
    let Some(room) = svc.store.room(&post.room).await? else {
        return Ok(());
    };
    let text = format!(
        "new {} from {} in {}; call room_read.",
        kind_word(post.kind),
        post.sender_name,
        post.room
    );
    let agents = svc.store.snapshot().await?.agents;
    let supervisors = super::supervisors_of(&svc.store, &room, &agents).await?;
    let recipients = match &post.to {
        Some(MemberRef::Agent(id)) => vec![id.clone()],
        Some(_) => vec![],
        // The room's tasks and its supervisor; a conversation that only looked in is not woken.
        None => {
            let mut ids = supervisors.clone();
            for id in super::present_agents(&svc.store, &room.id).await? {
                if !super::is_supervisor(&agents, &id) {
                    ids.push(id);
                }
            }
            ids.retain(|id| !matches!(&post.sender, MemberRef::Agent(s) if s == id));
            ids
        }
    };
    // A product room resumes no exited conversation; the post becomes a chat row instead. An operation room's
    // own conversation is resumed, as the spec has it.
    let mut parked = false;
    for id in &recipients {
        if room.kind == RoomKind::Product
            && supervisors.contains(id)
            && agents
                .iter()
                .any(|a| &a.id == id && a.status == Status::Stopped)
        {
            parked = true;
            continue;
        }
        if let Err(e) = notify(svc, id, &text).await {
            warn!(agent_id = %id.0, post = post.id, error = %one_line(&e.to_string()), "cannot wake a post's recipient; it is in the room");
        }
    }
    let to_supervisor = matches!(&post.to, Some(MemberRef::Agent(a)) if supervisors.contains(a));
    let kind = match post.kind {
        PostKind::Message if post.to == Some(MemberRef::Owner) || parked => {
            Some(TaskEventKind::RoomMessage)
        }
        PostKind::Ask if post.to == Some(MemberRef::Owner) || to_supervisor || parked => {
            Some(TaskEventKind::RoomAsk)
        }
        _ => None,
    };
    if let Some(kind) = kind {
        surface(svc, &room, post, kind, &post.text).await?;
    }
    Ok(())
}

/// Tells the member `text` (a line without the `Fleet Manager:` lead): an orchestrator task through its inbox and
/// its live agent, a supervisor conversation through a message. Anything else is not woken.
pub(crate) async fn notify(
    svc: &Arc<AgentService>,
    member: &AgentId,
    text: &str,
) -> Result<(), ServiceError> {
    let line = format!("{FROM} {text}");
    let live = match svc.store.task(member).await? {
        Some(t) if t.kind == AgentKind::Orchestrator => {
            let ev = FleetEvent {
                at: svc.options().clock.now(),
                step: 0,
                agent: t.id.clone(),
                kind: FleetEventKind::RoomPost,
                detail: text.to_string(),
            };
            svc.store.update_task(&t.id, |p| p.inbox.push(ev)).await?;
            t.lead.unwrap_or(t.id)
        }
        Some(_) => return Ok(()),
        None => match svc.find(member).await {
            Ok(a) if a.kind == AgentKind::Supervisor => a.id,
            _ => return Ok(()),
        },
    };
    svc.notices.push(&live, line);
    flush(svc, &live).await;
    Ok(())
}

/// Types the agent's queued notices, as one line, if it can take them now; else leaves them queued.
async fn flush(svc: &Arc<AgentService>, id: &AgentId) {
    let _l = svc.notices.lock.lock().await;
    let a = match svc.find(id).await {
        Ok(a) => a,
        // Gone for good: nobody is left to read them.
        Err(ServiceError::NotFound) => {
            svc.notices.take(id);
            return;
        }
        Err(e) => {
            warn!(agent_id = %id.0, error = %one_line(&e.to_string()), "cannot read the agent its notices are for");
            return;
        }
    };
    if svc.notices.awaiting(&a) {
        return;
    }
    let now = svc.options().clock.now();
    if a.kind == AgentKind::Supervisor {
        // Queued (starting) is let through on purpose: `message` waits for the input box before it types.
        if matches!(a.status, Status::Working | Status::NeedsYou) {
            return;
        }
        let lines = svc.notices.take(id);
        if lines.is_empty() {
            return;
        }
        svc.notices.set_awaiting(id, Some(now));
        // Off the caller: a resume can take seconds, and the poster carries on. Under the turn lock it checks again
        // that no turn has started (the owner's message may have got there first); if one has, the lines go back.
        let (svc, id) = (svc.clone(), id.clone());
        tokio::spawn(async move {
            let sent = Supervisor::new(svc.clone())
                .message_if_idle(&id, &lines.join(" "))
                .await;
            if let Err(e) = &sent {
                warn!(agent_id = %id.0, error = %one_line(&e.to_string()), "cannot tell the supervisor of a room post");
            }
            if !matches!(sent, Ok(true)) {
                let _l = svc.notices.lock.lock().await;
                svc.notices.restore(&id, lines);
                svc.notices.set_awaiting(&id, None);
            }
        });
        return;
    }
    // Only its input box at Review takes a line. A held tool call is a running turn: a waiting `room_ask`, a pending
    // `propose_breakdown` or a permission prompt (both NeedsYou) would get the line in the held turn or as a dialog's
    // answer. The holds are read as well as the status, which a late hook can flip back to Review.
    if a.status != Status::Review || svc.in_held_call(&a.id) {
        return;
    }
    let lines = svc.notices.take(id);
    if lines.is_empty() {
        return;
    }
    if svc.type_line(&a, &lines.join(" ")).await {
        svc.notices.set_awaiting(id, Some(now));
    } else {
        svc.notices.restore(id, lines);
    }
}

/// The agent's turn showed, or it stopped or exited: what it was typed has been taken, and its next Review may
/// type again.
async fn turn_seen(svc: &AgentService, id: &AgentId) {
    let _l = svc.notices.lock.lock().await;
    svc.notices.set_awaiting(id, None);
}

/// Puts the post in the owner's view: a `TaskEvent` routed to the sender's task's conversation, so it is a chat row
/// there and a toast. A post whose sender is no task (a supervisor's own message) is already in the owner's chat
/// with that supervisor, and `route_with` drops it.
async fn surface(
    svc: &AgentService,
    room: &RoomRow,
    post: &Post,
    kind: TaskEventKind,
    text: &str,
) -> Result<(), StoreError> {
    let MemberRef::Agent(sender) = &post.sender else {
        return Ok(());
    };
    let title = super::title(&svc.store, room).await?;
    let text = format!("{} in {title}: {text}", post.sender_name);
    route_with(&svc.store, svc.options().clock.now(), sender, kind, text).await
}

/// Puts each ask open for `REPOST_AFTER` to the owner once more, as a message; the post records it. Breakdown asks
/// (the card's Needs you already says so), asks in archived rooms and asks from ended tasks are not reposted.
pub async fn repost_stale_asks(svc: &AgentService) -> Result<(), ServiceError> {
    let now = svc.options().clock.now();
    for mut ask in svc.store.stale_asks(now - REPOST_AFTER).await? {
        // The task left its rooms when it ended; nobody is waiting on the answer any more.
        if let MemberRef::Agent(t) = &ask.sender {
            if svc.store.task(t).await?.is_some_and(|t| super::ended(&t)) {
                continue;
            }
        }
        if !svc.store.mark_reposted(ask.id, now).await? {
            continue;
        }
        let Some(room) = svc.store.room(&ask.room).await? else {
            continue;
        };
        ask.sender_name = svc.name_of(&room, &ask.sender).await?;
        let text = format!("unanswered for 30 minutes: {}", ask.text);
        surface(svc, &room, &ask, TaskEventKind::RoomMessage, &text).await?;
    }
    Ok(())
}

/// Types an agent's queued notices when it reaches Review, and clears its awaiting mark on any other status. A
/// lagged receiver may have missed a transition, so it tries every agent with notices waiting.
pub fn start_delivery_watcher(svc: Arc<AgentService>) -> JoinHandle<()> {
    let mut events = svc.store.subscribe();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(ev) => match ev.kind {
                    EventKind::StatusChanged {
                        id,
                        status: Status::Review,
                        ..
                    } => flush(&svc, &id).await,
                    EventKind::StatusChanged { id, .. } => turn_seen(&svc, &id).await,
                    EventKind::AgentRemoved { id } => {
                        turn_seen(&svc, &id).await;
                        svc.notices.take(&id);
                    }
                    _ => {}
                },
                // `flush` drops a mark the agent's status has moved past, so missed events strand nothing.
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    for id in svc.notices.waiting() {
                        flush(&svc, &id).await;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    })
}
