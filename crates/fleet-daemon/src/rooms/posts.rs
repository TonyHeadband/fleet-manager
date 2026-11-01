//! Posting, reading, asking and answering in rooms: the fleet tools behind `room_post`, `room_read`, `room_ask`
//! and `room_answer`.
//!
//! An orchestrator is in its rooms as its task (the id stays across orchestrator restarts), so whatever reaches it
//! goes to the task's live agent, `task.lead.unwrap_or(task.id)`. A supervisor conversation is in its operation
//! room as itself.
use super::deliver::{deliver, notify};
use crate::agents::{one_line, AgentService, ServiceError};
use crate::fleet::ops::detached;
use crate::store::RoomRow;
use fleet_api::{
    task_id, Agent, AgentId, AgentKind, BreakdownVersion, DecideBreakdown, MemberRef, Post,
    PostKind, RoomKind,
};
use std::sync::Arc;

/// What a held `room_ask` came back with.
#[derive(Debug, PartialEq, Eq)]
pub enum AskOutcome {
    Answered(String),
    /// The hold ran out; the ask stays open and its answer reaches the asker later.
    NoAnswerYet,
}

pub const NO_ANSWER_YET: &str = "no answer yet; call room_read later";
pub const SUPERVISOR_ASKS: &str = "the supervisor sends messages, not asks";
pub const SUPERVISOR_DECIDES: &str = "breakdown approval waits for the owner until autopilot";
pub const OWNER_DECIDES: &str = "breakdown approval waits for the owner";

/// `breakdown vN: <n> steps — <first step title>, …`
fn breakdown_line(v: &BreakdownVersion) -> String {
    let n = v.steps.len();
    let first = v.steps.first().map_or("", |s| s.title.as_str());
    let more = if n > 1 { ", …" } else { "" };
    let steps = if n == 1 { "step" } else { "steps" };
    format!("breakdown v{}: {n} {steps} — {first}{more}", v.version)
}

/// One post as `room_read` shows it: `<id> <sender> → <to>: [<kind>] <text>`. Senders and recipients read as the
/// values `to` takes, so a reply can be addressed from the line.
pub fn line(room: &RoomRow, p: &Post) -> String {
    let to = p.to.as_ref().map_or("all".to_string(), |m| label(room, m));
    format!(
        "{} {} → {to}: [{}] {}",
        p.id,
        label(room, &p.sender),
        kind_word(p.kind),
        p.text
    )
}

fn label(room: &RoomRow, m: &MemberRef) -> String {
    match m {
        MemberRef::Agent(a) if room.conversation.as_ref() == Some(a) => "supervisor".into(),
        m => m.key().to_string(),
    }
}

pub(crate) fn kind_word(k: PostKind) -> &'static str {
    match k {
        PostKind::Announcement => "announcement",
        PostKind::Message => "message",
        PostKind::Ask => "ask",
        PostKind::Answer => "answer",
    }
}

fn refuse(text: impl Into<String>) -> ServiceError {
    ServiceError::Refused(text.into())
}

impl AgentService {
    /// The agent, and the member it is in rooms as.
    async fn member_of(&self, caller: &AgentId) -> Result<(Agent, MemberRef), ServiceError> {
        let a = self.find(caller).await?;
        let id = match a.kind {
            AgentKind::Supervisor => a.id.clone(),
            _ => task_id(&a).clone(),
        };
        Ok((a, MemberRef::Agent(id)))
    }

    /// The room `who` (as `me`) means: `room` if it may use it, else its own operation room, else (an orchestrator)
    /// its product room. An orchestrator may use only the rooms it is in. A supervisor may use every room of its
    /// persona, each product room and its conversations' operation rooms, and joins one the first time it does, so
    /// its reads are tracked there too.
    async fn room_of(
        &self,
        who: &Agent,
        me: &MemberRef,
        room: Option<&str>,
    ) -> Result<RoomRow, ServiceError> {
        let mine = self.store.rooms_of(me.key()).await?;
        let supervisor = who.kind == AgentKind::Supervisor;
        let id = match room {
            Some(r) if mine.iter().any(|m| m == r) => r.to_string(),
            Some(r) if supervisor => {
                let found = self.store.room(r).await?;
                let theirs = found.as_ref().is_some_and(|f| match f.kind {
                    RoomKind::Product => true,
                    RoomKind::Operation => f.persona.as_deref() == Some(who.persona.as_str()),
                });
                if !theirs {
                    return Err(refuse(format!("room {r} is not one of your persona's")));
                }
                // Not `rooms::join`: a supervisor looking in must not reopen an archived room.
                self.store
                    .add_member(r, me.key(), self.opts.clock.now())
                    .await?;
                r.to_string()
            }
            Some(r) => return Err(refuse(format!("you are not in room {r}"))),
            None if supervisor => {
                let own = format!("operation:{}", who.id.0);
                mine.contains(&own)
                    .then_some(own)
                    .ok_or_else(|| refuse("you have no operation room yet; name a room"))?
            }
            None => mine
                .iter()
                .find(|r| r.starts_with("operation:"))
                .or(mine.first())
                .cloned()
                .ok_or_else(|| refuse("you are in no room"))?,
        };
        self.store
            .room(&id)
            .await?
            .ok_or_else(|| refuse(format!("no room {id}")))
    }

    /// `to` as a member of the room: `all` (None), `owner`, `supervisor` or a member's id. An orchestrator's agent
    /// id stands for its task.
    async fn recipient(&self, room: &RoomRow, to: &str) -> Result<Option<MemberRef>, ServiceError> {
        let id = match to {
            "all" => return Ok(None),
            "owner" => return Ok(Some(MemberRef::Owner)),
            "supervisor" => {
                return room
                    .conversation
                    .clone()
                    .map(|c| Some(MemberRef::Agent(c)))
                    .ok_or_else(|| refuse("this room has no supervisor"))
            }
            id => AgentId(id.to_string()),
        };
        self.agent_recipient(room, id, to).await.map(Some)
    }

    /// The agent as a member of the room, refused (as `shown`) unless it is in it. An orchestrator's agent id stands
    /// for its task.
    async fn agent_recipient(
        &self,
        room: &RoomRow,
        id: AgentId,
        shown: &str,
    ) -> Result<MemberRef, ServiceError> {
        let id = match self.find(&id).await {
            Ok(a) if a.kind == AgentKind::Orchestrator => task_id(&a).clone(),
            _ => id,
        };
        let present = self
            .store
            .room_members(&room.id)
            .await?
            .iter()
            .any(|m| m.member == id.0 && m.left_at.is_none());
        if !present {
            return Err(refuse(format!("{shown} is not in this room")));
        }
        Ok(MemberRef::Agent(id))
    }

    /// The sender as people read it.
    pub(crate) async fn name_of(
        &self,
        room: &RoomRow,
        m: &MemberRef,
    ) -> Result<String, ServiceError> {
        Ok(match m {
            MemberRef::Owner => "owner".into(),
            MemberRef::Daemon => "Fleet Manager".into(),
            MemberRef::Agent(a) if room.conversation.as_ref() == Some(a) => "supervisor".into(),
            MemberRef::Agent(a) => match self.store.task(a).await? {
                Some(t) => t.title.filter(|t| !t.is_empty()).unwrap_or(t.name),
                None => a.0.clone(),
            },
        })
    }

    /// Names, stores, broadcasts and logs the post; returns it with its id.
    async fn store_post(&self, room: &RoomRow, mut p: Post) -> Result<Post, ServiceError> {
        p.sender_name = self.name_of(room, &p.sender).await?;
        let ev = self.store.add_post(&p).await?;
        if let fleet_api::EventKind::RoomPost { post } = ev.kind {
            p = post;
        }
        log_post(&p);
        Ok(p)
    }

    /// `store_post`, then wakes whoever the post is for. A wake that fails does not undo the post: it is in the room
    /// for `room_read`.
    async fn publish(self: &Arc<Self>, room: &RoomRow, p: Post) -> Result<Post, ServiceError> {
        let p = self.store_post(room, p).await?;
        if let Err(e) = deliver(self, &p).await {
            tracing::warn!(post = p.id, error = %one_line(&e.to_string()), "cannot wake the post's recipients");
        }
        Ok(p)
    }

    fn new_post(
        &self,
        room: &RoomRow,
        sender: MemberRef,
        kind: PostKind,
        text: &str,
    ) -> Result<Post, ServiceError> {
        if text.trim().is_empty() {
            return Err(ServiceError::EmptyText);
        }
        Ok(Post {
            id: 0,
            room: room.id.clone(),
            sender_name: String::new(),
            sender,
            to: None,
            kind,
            reply_to: None,
            text: text.to_string(),
            at: self.opts.clock.now(),
            answered_at: None,
            breakdown_version: None,
        })
    }

    /// `room_post`: an announcement or a message, to `to` (everyone by default). Returns the post's id.
    pub async fn room_post(
        self: &Arc<Self>,
        caller: &AgentId,
        kind: PostKind,
        text: &str,
        to: Option<&str>,
        room: Option<&str>,
    ) -> Result<i64, ServiceError> {
        if !matches!(kind, PostKind::Announcement | PostKind::Message) {
            return Err(refuse("room_post sends an announcement or a message"));
        }
        let (who, me) = self.member_of(caller).await?;
        let room = self.room_of(&who, &me, room).await?;
        let mut p = self.new_post(&room, me, kind, text)?;
        p.to = self.recipient(&room, to.unwrap_or("all")).await?;
        Ok(self.publish(&room, p).await?.id)
    }

    /// The owner's post from the Rooms widget: an announcement or a message, to one member or everyone. A message
    /// wakes its recipients as an agent's does.
    pub async fn owner_post(
        self: &Arc<Self>,
        room: &str,
        kind: PostKind,
        to: Option<&MemberRef>,
        text: &str,
    ) -> Result<Post, ServiceError> {
        if !matches!(kind, PostKind::Announcement | PostKind::Message) {
            return Err(refuse("the owner posts an announcement or a message"));
        }
        let room = self.store.room(room).await?.ok_or(ServiceError::NotFound)?;
        let mut p = self.new_post(&room, MemberRef::Owner, kind, text)?;
        p.to = match to {
            None => None,
            // Not `recipient`: that reads `owner` and `all` as words, and here they would be agent ids.
            Some(MemberRef::Agent(a)) => Some(self.agent_recipient(&room, a.clone(), &a.0).await?),
            Some(m) => return Err(refuse(format!("{} is not a member to post to", m.key()))),
        };
        self.publish(&room, p).await
    }

    /// The owner's view of a room: up to `limit` posts before `before` (the latest when None), oldest first, each
    /// with its sender's name. Whatever it returns is marked read for the owner, who joins the room for that.
    pub async fn owner_read(
        &self,
        room: &str,
        before: Option<i64>,
        limit: u32,
    ) -> Result<Vec<Post>, ServiceError> {
        let room = self.store.room(room).await?.ok_or(ServiceError::NotFound)?;
        let mut posts = self.store.room_posts(&room.id, before, limit).await?;
        let mut names: std::collections::HashMap<MemberRef, String> = Default::default();
        for p in &mut posts {
            p.sender_name = match names.get(&p.sender) {
                Some(n) => n.clone(),
                None => {
                    let n = self.name_of(&room, &p.sender).await?;
                    names.insert(p.sender.clone(), n.clone());
                    n
                }
            };
        }
        if let Some(last) = posts.last() {
            let owner = MemberRef::Owner.key();
            self.store
                .add_member(&room.id, owner, self.opts.clock.now())
                .await?;
            self.store.set_read_upto(&room.id, owner, last.id).await?;
        }
        Ok(posts)
    }

    /// `room_read`: the posts the caller has not seen in the room, which are then marked read.
    pub async fn room_read(
        &self,
        caller: &AgentId,
        room: Option<&str>,
    ) -> Result<(RoomRow, Vec<Post>), ServiceError> {
        let (who, me) = self.member_of(caller).await?;
        let room = self.room_of(&who, &me, room).await?;
        let upto = self
            .store
            .room_members(&room.id)
            .await?
            .into_iter()
            .find(|m| m.member == me.key())
            .map_or(0, |m| m.read_upto);
        let mut posts = self.store.posts_after(&room.id, me.key(), upto).await?;
        for p in &mut posts {
            p.sender_name = self.name_of(&room, &p.sender).await?;
        }
        if let Some(last) = posts.last() {
            self.store
                .set_read_upto(&room.id, me.key(), last.id)
                .await?;
        }
        Ok((room, posts))
    }

    /// `room_ask`: an orchestrator's ask to the supervisor or the owner, held until it is answered or the hold runs
    /// out. The ask is stored either way; an answer that finds no held call reaches the asker as a fleet notice.
    pub async fn room_ask(
        self: &Arc<Self>,
        caller: &AgentId,
        to: &str,
        text: &str,
        room: Option<&str>,
    ) -> Result<(i64, AskOutcome), ServiceError> {
        let (who, me) = self.member_of(caller).await?;
        if who.kind == AgentKind::Supervisor {
            return Err(refuse(SUPERVISOR_ASKS));
        }
        if !matches!(to, "supervisor" | "owner") {
            return Err(refuse("an ask goes to the supervisor or the owner"));
        }
        let room = self.room_of(&who, &me, room).await?;
        let mut p = self.new_post(&room, me, PostKind::Ask, text)?;
        p.to = self.recipient(&room, to).await?;
        // Detached, so a client that hangs up cannot stop it between storing the ask and registering its call.
        let svc = self.clone();
        let (id, mut rx) = detached(async move {
            let _ask = svc.asks.lock.lock().await;
            let p = svc.publish(&room, p).await?;
            Ok((p.id, svc.asks.hold(p.id)))
        })
        .await?;
        if let Ok(Ok(answer)) = tokio::time::timeout(self.opts.repo_request_hold, &mut rx).await {
            return Ok((id, AskOutcome::Answered(answer)));
        }
        // Under the lock an answer has either been handed over already (it is in `rx`) or will find no call and be
        // typed as a notice: never both, never neither.
        let _ask = self.asks.lock.lock().await;
        self.asks.forget(id);
        Ok(match rx.try_recv() {
            Ok(answer) => (id, AskOutcome::Answered(answer)),
            Err(_) => (id, AskOutcome::NoAnswerYet),
        })
    }

    /// `room_answer` from an agent.
    pub async fn room_answer_from(
        self: &Arc<Self>,
        caller: &AgentId,
        ask: i64,
        text: &str,
    ) -> Result<bool, ServiceError> {
        let (_, me) = self.member_of(caller).await?;
        self.room_answer(&me, ask, text).await
    }

    /// Answers the ask as `by`: the owner, or a member of its room other than the asker. False when it was answered
    /// already. The answer goes to the held call, or else to the asker as a fleet notice.
    pub async fn room_answer(
        self: &Arc<Self>,
        by: &MemberRef,
        ask: i64,
        text: &str,
    ) -> Result<bool, ServiceError> {
        let q = self
            .store
            .post(ask)
            .await?
            .filter(|p| p.kind == PostKind::Ask)
            .ok_or_else(|| refuse(format!("no ask {ask}")))?;
        let room = self
            .store
            .room(&q.room)
            .await?
            .ok_or_else(|| refuse(format!("no room {}", q.room)))?;
        if let Some(v) = q.breakdown_version {
            return self.decide_breakdown_ask(by, &q, v, text).await;
        }
        if by != &MemberRef::Owner {
            if by == &q.sender {
                return Err(refuse("you cannot answer your own ask"));
            }
            let present = self
                .store
                .room_members(&room.id)
                .await?
                .iter()
                .any(|m| m.member == by.key() && m.left_at.is_none());
            if !present {
                return Err(refuse(format!("you are not in room {}", room.id)));
            }
        }
        let mut answer = self.new_post(&room, by.clone(), PostKind::Answer, text)?;
        answer.to = Some(q.sender.clone());
        answer.reply_to = Some(ask);
        answer.sender_name = self.name_of(&room, by).await?;
        let held = {
            let _ask = self.asks.lock.lock().await;
            if !self.store.answer_ask(ask, &answer).await? {
                return Ok(false);
            }
            self.asks.resolve(ask, text)
        };
        log_post(&answer);
        if !held {
            self.notice_answer(&q, &answer).await;
        }
        Ok(true)
    }

    /// An answer to a breakdown's ask is the owner's decision on that version: `approve` approves it, any other
    /// text asks for changes with the text as the comment. `decide_breakdown` marks the ask answered. False when the
    /// version was decided already.
    async fn decide_breakdown_ask(
        &self,
        by: &MemberRef,
        q: &Post,
        version: u32,
        text: &str,
    ) -> Result<bool, ServiceError> {
        match by {
            MemberRef::Owner => {}
            MemberRef::Agent(a)
                if self
                    .find(a)
                    .await
                    .is_ok_and(|x| x.kind == AgentKind::Supervisor) =>
            {
                return Err(refuse(SUPERVISOR_DECIDES))
            }
            _ => return Err(refuse(OWNER_DECIDES)),
        }
        let MemberRef::Agent(task) = &q.sender else {
            return Err(refuse(format!("no ask {}", q.id)));
        };
        if text.trim().is_empty() {
            return Err(ServiceError::EmptyText);
        }
        let latest = self
            .store
            .task(task)
            .await?
            .and_then(|p| p.breakdowns.last().map(|v| v.version));
        if let Some(latest) = latest.filter(|l| *l != version) {
            return Err(refuse(format!(
                "breakdown v{version} is stale: v{latest} is the latest"
            )));
        }
        let approve = text.trim().eq_ignore_ascii_case("approve");
        let d = DecideBreakdown {
            approve,
            comment: (!approve).then(|| text.to_string()),
            version: Some(version),
        };
        match self.decide_breakdown_as(task, d, Some(text)).await {
            Ok(()) => Ok(true),
            Err(ServiceError::NoPendingBreakdown) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Puts a proposed breakdown version to the task's room as an ask: its operation room, addressed to the
    /// supervisor, else its product room, to the owner. Only the owner answers it. An older version's ask still open
    /// is closed as superseded.
    pub(crate) async fn post_breakdown_ask(
        &self,
        task: &AgentId,
        v: &BreakdownVersion,
    ) -> Result<(), ServiceError> {
        let me = MemberRef::Agent(task.clone());
        for q in self.store.open_breakdown_asks(me.key()).await? {
            let superseded = format!("superseded by v{}", v.version);
            self.close_breakdown_ask(&q, &MemberRef::Daemon, &superseded)
                .await?;
        }
        let mine = self.store.rooms_of(me.key()).await?;
        let Some(id) = mine
            .iter()
            .find(|r| r.starts_with("operation:"))
            .or(mine.first())
        else {
            return Ok(());
        };
        let Some(room) = self.store.room(id).await? else {
            return Ok(());
        };
        let mut p = self.new_post(&room, me, PostKind::Ask, &breakdown_line(v))?;
        p.to = Some(
            room.conversation
                .clone()
                .map_or(MemberRef::Owner, MemberRef::Agent),
        );
        p.breakdown_version = Some(v.version);
        self.store_post(&room, p).await?;
        Ok(())
    }

    /// Marks the task's open ask for `version` answered by `by` with `text`, once a decision was made elsewhere.
    pub(crate) async fn close_breakdown_asks_for(
        &self,
        task: &AgentId,
        version: u32,
        by: &MemberRef,
        text: &str,
    ) -> Result<(), ServiceError> {
        for q in self.store.open_breakdown_asks(&task.0).await? {
            if q.breakdown_version == Some(version) {
                self.close_breakdown_ask(&q, by, text).await?;
            }
        }
        Ok(())
    }

    /// Stores the answer to a breakdown ask. Nobody is told: the decision reaches the orchestrator as
    /// `propose_breakdown`'s answer or a fleet event.
    async fn close_breakdown_ask(
        &self,
        q: &Post,
        by: &MemberRef,
        text: &str,
    ) -> Result<(), ServiceError> {
        let Some(room) = self.store.room(&q.room).await? else {
            return Ok(());
        };
        let mut answer = self.new_post(&room, by.clone(), PostKind::Answer, text)?;
        answer.to = Some(q.sender.clone());
        answer.reply_to = Some(q.id);
        answer.sender_name = self.name_of(&room, by).await?;
        if self.store.answer_ask(q.id, &answer).await? {
            log_post(&answer);
        }
        Ok(())
    }

    /// Tells the asker the answer as a fleet notice: typed now if it is at its input box, else when it next is, and
    /// in its task's inbox for `wait_for_fleet`.
    async fn notice_answer(self: &Arc<Self>, q: &Post, answer: &Post) {
        let MemberRef::Agent(asker) = &q.sender else {
            return;
        };
        let text = format!(
            "{} answered your ask {}: {}",
            answer.sender_name,
            q.id,
            one_line(&answer.text)
        );
        if let Err(e) = notify(self, asker, &text).await {
            tracing::warn!(ask = q.id, error = %one_line(&e.to_string()), "cannot tell the asker the answer; it is in the room");
        }
    }
}

fn log_post(p: &Post) {
    tracing::info!(
        room = %p.room,
        sender = p.sender.key(),
        to = p.to.as_ref().map_or("all", |t| t.key()),
        kind = kind_word(p.kind),
        "room post"
    );
}
