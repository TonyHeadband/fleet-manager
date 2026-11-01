//! Posting, reading, asking and answering in rooms: the fleet tools behind `room_post`, `room_read`, `room_ask`
//! and `room_answer`.
//!
//! An orchestrator is in its rooms as its task (the id stays across orchestrator restarts), so whatever reaches it
//! goes to the task's live agent, `task.lead.unwrap_or(task.id)`. A supervisor conversation is in its operation
//! room as itself.
use crate::agents::{one_line, AgentService, ServiceError};
use crate::fleet::ops::detached;
use crate::store::RoomRow;
use fleet_api::{task_id, Agent, AgentId, AgentKind, MemberRef, Post, PostKind, RoomKind, Status};
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

fn kind_word(k: PostKind) -> &'static str {
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
            return Err(refuse(format!("{to} is not in this room")));
        }
        Ok(Some(MemberRef::Agent(id)))
    }

    /// The sender as people read it.
    async fn name_of(&self, room: &RoomRow, m: &MemberRef) -> Result<String, ServiceError> {
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
    async fn publish(&self, room: &RoomRow, mut p: Post) -> Result<Post, ServiceError> {
        p.sender_name = self.name_of(room, &p.sender).await?;
        let ev = self.store.add_post(&p).await?;
        if let fleet_api::EventKind::RoomPost { post } = ev.kind {
            p = post;
        }
        log_post(&p);
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
        &self,
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
        &self,
        caller: &AgentId,
        ask: i64,
        text: &str,
    ) -> Result<bool, ServiceError> {
        let (_, me) = self.member_of(caller).await?;
        self.room_answer(&me, ask, text).await
    }

    /// Answers the ask as `by`: the owner, or a member of its room other than the asker. False when it was answered
    /// already. The answer goes to the held call, or else is typed into the asker's live agent as a fleet notice.
    pub async fn room_answer(
        &self,
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

    /// Types the answer into the asker's live agent. One that cannot take a line now finds it with `room_read`.
    async fn notice_answer(&self, q: &Post, answer: &Post) {
        let MemberRef::Agent(asker) = &q.sender else {
            return;
        };
        let live = match self.store.task(asker).await {
            Ok(Some(t)) => t.lead.unwrap_or(t.id),
            _ => asker.clone(),
        };
        let line = format!(
            "Fleet Manager: {} answered your ask {}: {}",
            answer.sender_name,
            q.id,
            one_line(&answer.text)
        );
        // Never into a running turn: a Working agent gets it from Task 4's queue instead.
        let typed = match self.find(&live).await {
            Ok(a) if a.status != Status::Working => self.type_line(&a, &line).await,
            _ => false,
        };
        if !typed {
            self.undelivered_notice(&live, q.id, &line);
        }
    }

    /// A notice the asker could not take now. It is not lost: the answer is in the room for its `room_read`.
    // Task 4 queues this: route it into the agent's pending queue (typed when it next reaches Review) and the
    // task's `inbox` for `wait_for_fleet`.
    fn undelivered_notice(&self, agent: &AgentId, ask: i64, line: &str) {
        tracing::info!(agent_id = %agent.0, ask, %line, "answer not typed; it waits in the room");
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
