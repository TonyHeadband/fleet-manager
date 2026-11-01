//! An agent's whole chat: every Claude session it has had, built once and then followed as the last one grows.
use crate::agents::{transcript_path, AgentService, PROMPT_FILE};
use crate::chat::{blocks_before, Builder, Tail};
use crate::personas::PersonaEnv;
use fleet_api::{Agent, AgentId, AgentKind, ChatItem, ChatPage, MarkerKind, PromptFrom};
use std::path::Path;
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::warn;

/// Items in the stream's first page, and the history page's default size.
pub const PAGE: usize = 200;
/// How often a follower reads its transcript when no hook has woken it.
pub const POLL: Duration = Duration::from_secs(2);

/// Wakes an agent's chat followers when its hooks report something, so they read the transcript at once.
#[derive(Clone)]
pub struct ChatWake(broadcast::Sender<AgentId>);

impl Default for ChatWake {
    fn default() -> Self {
        Self(broadcast::channel(64).0)
    }
}

impl ChatWake {
    pub fn poke(&self, agent: &AgentId) {
        let _ = self.0.send(agent.clone());
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AgentId> {
        self.0.subscribe()
    }
}

/// Returns when `agent` is poked, or when pokes were missed (any of them may have been its).
pub async fn woken(rx: &mut broadcast::Receiver<AgentId>, agent: &AgentId) {
    loop {
        match rx.recv().await {
            Ok(id) if &id == agent => return,
            Ok(_) => {}
            Err(broadcast::error::RecvError::Lagged(_)) => return,
            Err(broadcast::error::RecvError::Closed) => std::future::pending().await,
        }
    }
}

struct Session {
    id: String,
    /// None until the transcript file is found.
    tail: Option<Tail>,
}

/// Every item of an agent's chat so far, indexed by `seq`.
pub struct Chat {
    env: Option<PersonaEnv>,
    builder: Builder,
    sessions: Vec<Session>,
    items: Vec<ChatItem>,
    ended: bool,
}

impl Chat {
    pub fn new(
        env: Option<PersonaEnv>,
        prompt_text: Option<String>,
        prompt_from: PromptFrom,
    ) -> Self {
        Chat {
            env,
            builder: Builder::new(prompt_text, prompt_from),
            sessions: Vec::new(),
            items: Vec::new(),
            ended: false,
        }
    }

    /// The chat of `agent` before any transcript is read. A specialist's PROMPT.md is its orchestrator's.
    pub async fn open(svc: &AgentService, agent: &Agent) -> Self {
        let env = if agent.persona.is_empty() {
            None
        } else {
            svc.personas().env(&agent.persona).await.ok()
        };
        let prompt = tokio::fs::read_to_string(Path::new(&agent.workdir).join(PROMPT_FILE))
            .await
            .ok();
        let from = match agent.kind {
            AgentKind::Specialist => PromptFrom::System,
            AgentKind::Solo | AgentKind::Orchestrator => PromptFrom::Owner,
        };
        Chat::new(env, prompt, from)
    }

    /// Reads what the agent's `sessions` (oldest first) added since the last call; returns the new and updated items.
    /// A session not seen before ends the one before it with a `NewSession` divider. Blocking file IO.
    pub fn advance(&mut self, sessions: &[(String, i64)]) -> Vec<ChatItem> {
        let mut fresh = Vec::new();
        for (id, at) in sessions {
            if self.sessions.iter().any(|s| &s.id == id) {
                continue;
            }
            if !self.sessions.is_empty() {
                fresh.extend(self.read_last());
                fresh.extend(self.builder.close_open());
                fresh.push(self.builder.new_session(*at));
            }
            self.sessions.push(Session {
                id: id.clone(),
                tail: None,
            });
        }
        fresh.extend(self.read_last());
        self.merge(&fresh);
        fresh
    }

    /// Closes the chat of a stopped agent: its open calls fail and a `SessionEnded` marker dated `at` follows. Once.
    pub fn end(&mut self, at: i64) -> Vec<ChatItem> {
        if self.ended {
            return Vec::new();
        }
        self.ended = true;
        let mut fresh = self.builder.close_open();
        fresh.push(self.builder.marker(MarkerKind::SessionEnded, at));
        self.merge(&fresh);
        fresh
    }

    /// Up to `limit` items before `before` (all of them when None), oldest first.
    pub fn page(&self, before: Option<u64>, limit: usize) -> ChatPage {
        let end = before.map_or(self.items.len(), |b| (b as usize).min(self.items.len()));
        let start = end.saturating_sub(limit);
        ChatPage {
            items: self.items[start..end].to_vec(),
            earlier: blocks_before(&self.items[..start]),
        }
    }

    /// Only the newest session is read: an older one ended when the next began.
    fn read_last(&mut self) -> Vec<ChatItem> {
        let Some(session) = self.sessions.last_mut() else {
            return Vec::new();
        };
        if session.tail.is_none() {
            session.tail = self
                .env
                .as_ref()
                .and_then(|env| transcript_path(env, &session.id))
                .map(Tail::new);
        }
        let Some(tail) = session.tail.as_mut() else {
            return Vec::new();
        };
        match tail.read_new() {
            Ok(lines) => lines.iter().flat_map(|l| self.builder.line(l)).collect(),
            Err(e) => {
                warn!(session = %session.id, error = %e, "cannot read the transcript");
                Vec::new()
            }
        }
    }

    /// Seqs are minted in order from 0, so an item's seq is its index; an update replaces the item it names.
    fn merge(&mut self, fresh: &[ChatItem]) {
        for item in fresh {
            let at = item.seq as usize;
            if at < self.items.len() {
                self.items[at] = item.clone();
            } else {
                debug_assert_eq!(at, self.items.len());
                self.items.push(item.clone());
            }
        }
    }
}

/// `advance`, then `end` when `ended_at` is set, on a blocking thread; None if that thread panicked.
pub async fn step(
    mut chat: Chat,
    sessions: Vec<(String, i64)>,
    ended_at: Option<i64>,
) -> Option<(Chat, Vec<ChatItem>)> {
    tokio::task::spawn_blocking(move || {
        let mut fresh = chat.advance(&sessions);
        if let Some(at) = ended_at {
            fresh.extend(chat.end(at));
        }
        (chat, fresh)
    })
    .await
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fleet_api::ChatBody;

    fn prompt(n: u64) -> ChatItem {
        ChatItem {
            seq: n,
            at: 0,
            body: ChatBody::Prompt {
                text: n.to_string(),
                from: PromptFrom::Owner,
            },
        }
    }

    #[test]
    fn pages_count_the_blocks_before_them() {
        let mut chat = Chat::new(None, None, PromptFrom::Owner);
        chat.merge(&(0..10).map(prompt).collect::<Vec<_>>());
        let p = chat.page(Some(7), 3);
        assert_eq!(p.items.iter().map(|i| i.seq).collect::<Vec<_>>(), [4, 5, 6]);
        assert_eq!(p.earlier, 4);
        assert_eq!(chat.page(None, 100).items.len(), 10);
        assert_eq!(chat.page(Some(99), 2).earlier, 8);
    }

    #[tokio::test]
    async fn a_poke_wakes_only_its_agents_followers() {
        let wake = ChatWake::default();
        let mut rx = wake.subscribe();
        let (a, b) = (AgentId("a".into()), AgentId("b".into()));
        wake.poke(&b);
        wake.poke(&a);
        tokio::time::timeout(Duration::from_secs(1), woken(&mut rx, &a))
            .await
            .unwrap();
        wake.poke(&b);
        assert!(
            tokio::time::timeout(Duration::from_millis(50), woken(&mut rx, &a))
                .await
                .is_err()
        );
    }
}
