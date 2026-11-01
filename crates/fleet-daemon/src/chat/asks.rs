//! The prompts agents wait on. Claude sends every permission prompt, AskUserQuestion included, as a PermissionRequest
//! hook; the daemon holds that request open and answers it with the owner's choice from the chat. Decision shapes are
//! the ones confirmed against Claude Code 2.1.316 (`tests/fixtures/transcripts/README.md`).
use fleet_api::{AgentId, AskAnswer, AskKind, AskOption, AskQuestion, ChatAsk};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast, oneshot};

/// The id of every `Screen` ask: it is answered through the reply box, never through this registry.
pub const SCREEN_ID: &str = "screen";
const DENY_MESSAGE: &str = "The owner denied this.";
/// Answered ask ids remembered so a late second answer gets 409 rather than 404.
const ANSWERED_KEPT: usize = 256;

struct Pending {
    ask: ChatAsk,
    input: Value,
    /// Claude's suggested `addRules`, rescoped to the session: what "allow for the session" sends back.
    session_rules: Vec<Value>,
    /// None once answered.
    reply: Option<oneshot::Sender<Value>>,
}

#[derive(Default)]
struct Inner {
    pending: HashMap<AgentId, Pending>,
    answered: VecDeque<String>,
    /// Rises with every change, so a reader of `get` can tell which queued changes it has already seen.
    version: u64,
}

/// A change of an agent's ask: the registry's version after it, the agent, and its ask now.
pub type Change = (u64, AgentId, Option<ChatAsk>);

/// One held prompt per agent: Claude asks one thing at a time.
#[derive(Clone)]
pub struct Asks {
    inner: Arc<Mutex<Inner>>,
    /// Every change of an agent's ask, so a stream sends each one, the answered state included.
    changes: broadcast::Sender<Change>,
}

impl Default for Asks {
    fn default() -> Self {
        Asks {
            inner: Arc::default(),
            changes: broadcast::channel(64).0,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum AnswerError {
    Unknown,
    Answered,
    /// The `Screen` ask: answered through the reply box.
    Screen,
    Bad(String),
}

/// A PermissionRequest held open. Dropping it (the hook timed out, or Claude closed the connection) forgets an
/// unanswered ask, so the chat falls back to the screen.
pub struct Held {
    asks: Asks,
    agent: AgentId,
    id: String,
    rx: oneshot::Receiver<Value>,
}

impl Held {
    /// The decision to send Claude, or `{}` (Claude keeps its own prompt) when nobody answers within `max`.
    pub async fn wait(mut self, max: Duration) -> Value {
        match tokio::time::timeout(max, &mut self.rx).await {
            Ok(Ok(decision)) => decision,
            _ => json!({}),
        }
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        let mut inner = self.asks.inner.lock().expect("asks poisoned");
        let ours = inner
            .pending
            .get(&self.agent)
            .is_some_and(|p| p.ask.id == self.id && p.ask.answered.is_none());
        if ours {
            inner.pending.remove(&self.agent);
            self.asks.changed(&mut inner, &self.agent, None);
        }
    }
}

impl Asks {
    pub fn subscribe(&self) -> broadcast::Receiver<Change> {
        self.changes.subscribe()
    }

    /// The agent's ask, and the version it was read at: changes up to that version are already in it.
    pub fn get(&self, agent: &AgentId) -> (u64, Option<ChatAsk>) {
        let inner = self.inner.lock().expect("asks poisoned");
        (
            inner.version,
            inner.pending.get(agent).map(|p| p.ask.clone()),
        )
    }

    /// Records the PermissionRequest `body` as `agent`'s ask, replacing any earlier one. None when the body is not one
    /// the chat can show (a question it cannot read), so the hook answers at once and Claude asks in its TUI.
    pub fn hold(&self, agent: &AgentId, body: &Value) -> Option<Held> {
        let (kind, input, session_rules) = parse(body)?;
        let ask = ChatAsk {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            answered: None,
        };
        let (tx, rx) = oneshot::channel();
        let mut inner = self.inner.lock().expect("asks poisoned");
        inner.pending.insert(
            agent.clone(),
            Pending {
                ask: ask.clone(),
                input,
                session_rules,
                reply: Some(tx),
            },
        );
        self.changed(&mut inner, agent, Some(ask.clone()));
        Some(Held {
            asks: self.clone(),
            agent: agent.clone(),
            id: ask.id,
            rx,
        })
    }

    /// Sends the decision for `answer` to the held hook; the ask stays, answered, until `clear`.
    pub fn answer(&self, agent: &AgentId, id: &str, answer: &AskAnswer) -> Result<(), AnswerError> {
        let mut guard = self.inner.lock().expect("asks poisoned");
        let inner = &mut *guard;
        let Some(p) = inner.pending.get_mut(agent).filter(|p| p.ask.id == id) else {
            return Err(if id == SCREEN_ID {
                AnswerError::Screen
            } else if inner.answered.iter().any(|a| a == id) {
                AnswerError::Answered
            } else {
                AnswerError::Unknown
            });
        };
        if p.reply.is_none() {
            return Err(AnswerError::Answered);
        }
        let (decision, label) = decide(p, answer)?;
        let wrapped = json!({"hookSpecificOutput": {
            "hookEventName": "PermissionRequest",
            "decision": decision,
        }});
        let sent = p.reply.take().is_some_and(|r| r.send(wrapped).is_ok());
        if !sent {
            // The hook went away while this answer was being made.
            inner.pending.remove(agent);
            self.changed(inner, agent, None);
            return Err(AnswerError::Unknown);
        }
        p.ask.answered = Some(label);
        let ask = p.ask.clone();
        if inner.answered.len() == ANSWERED_KEPT {
            inner.answered.pop_front();
        }
        inner.answered.push_back(ask.id.clone());
        self.changed(inner, agent, Some(ask));
        Ok(())
    }

    /// The agent moved on (a tool ran, a turn ended): its prompt, answered or not, is no longer waiting.
    pub fn clear(&self, agent: &AgentId) {
        let mut inner = self.inner.lock().expect("asks poisoned");
        if inner.pending.remove(agent).is_some() {
            self.changed(&mut inner, agent, None);
        }
    }

    /// Called with the lock held, so versions go out in order.
    fn changed(&self, inner: &mut Inner, agent: &AgentId, ask: Option<ChatAsk>) {
        inner.version += 1;
        let _ = self.changes.send((inner.version, agent.clone(), ask));
    }
}

fn parse(body: &Value) -> Option<(AskKind, Value, Vec<Value>)> {
    let tool = body.get("tool_name")?.as_str()?.to_string();
    let input = body.get("tool_input").cloned().unwrap_or_else(|| json!({}));
    if tool == "AskUserQuestion" {
        return Some((
            AskKind::Question {
                questions: questions(&input)?,
            },
            input,
            Vec::new(),
        ));
    }
    // Only `addRules` is confirmed to stick for the session; a folder suggestion did not stop the next prompt.
    let session_rules: Vec<Value> = body
        .get("permission_suggestions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|s| s.get("type").and_then(Value::as_str) == Some("addRules"))
        .map(|s| {
            let mut s = s.clone();
            s["destination"] = json!("session");
            s
        })
        .collect();
    let option = |id: &str, label: &str| AskOption {
        id: id.into(),
        label: label.into(),
    };
    let mut options = vec![option("allow", "Allow once")];
    if !session_rules.is_empty() {
        options.push(option("allow_session", "Allow for this session"));
    }
    options.push(option("deny", "Deny"));
    Some((
        AskKind::Permission {
            tool,
            input: input.clone(),
            options,
        },
        input,
        session_rules,
    ))
}

fn questions(input: &Value) -> Option<Vec<AskQuestion>> {
    let list = input.get("questions")?.as_array()?;
    if list.is_empty() {
        return None;
    }
    list.iter()
        .enumerate()
        .map(|(q, v)| {
            let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
            let options = v
                .get("options")?
                .as_array()?
                .iter()
                .enumerate()
                .map(|(o, opt)| {
                    Some(AskOption {
                        id: format!("{q}.{o}"),
                        label: opt.get("label")?.as_str()?.to_string(),
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            Some(AskQuestion {
                header: text("header").unwrap_or_default(),
                question: text("question")?,
                options,
                multi_select: v
                    .get("multiSelect")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect()
}

/// The hook's `decision` for `answer`, and the label the chat shows for it.
fn decide(p: &Pending, answer: &AskAnswer) -> Result<(Value, String), AnswerError> {
    match &p.ask.kind {
        AskKind::Permission { options, .. } => {
            let label = options
                .iter()
                .find(|o| o.id == answer.choice)
                .map(|o| o.label.clone())
                .ok_or_else(|| AnswerError::Bad(format!("no option {:?}", answer.choice)))?;
            let decision = match answer.choice.as_str() {
                "allow" => json!({"behavior": "allow"}),
                "allow_session" => {
                    json!({"behavior": "allow", "updatedPermissions": p.session_rules})
                }
                _ => json!({"behavior": "deny", "message": DENY_MESSAGE}),
            };
            Ok((decision, label))
        }
        AskKind::Question { questions } => {
            let answers = answers(questions, answer)?;
            let label = answers
                .iter()
                .map(|(_, a)| a.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            let mut input = p.input.clone();
            input["answers"] = Value::Object(
                answers
                    .into_iter()
                    .map(|(q, a)| (q, Value::String(a)))
                    .collect::<Map<_, _>>(),
            );
            Ok((json!({"behavior": "allow", "updatedInput": input}), label))
        }
        AskKind::Screen { .. } => Err(AnswerError::Screen),
    }
}

/// Claude's `answers`: keyed by question text, a multi-select's labels joined with `", "`, free text as given. The
/// text answers the one question no chosen option answers.
fn answers(
    questions: &[AskQuestion],
    answer: &AskAnswer,
) -> Result<Vec<(String, String)>, AnswerError> {
    let bad = |m: String| AnswerError::Bad(m);
    let mut picked: Vec<Vec<usize>> = vec![Vec::new(); questions.len()];
    for id in answer
        .choice
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let (q, o) = id
            .split_once('.')
            .and_then(|(q, o)| Some((q.parse::<usize>().ok()?, o.parse::<usize>().ok()?)))
            .filter(|&(q, o)| questions.get(q).is_some_and(|x| o < x.options.len()))
            .ok_or_else(|| bad(format!("no option {id:?}")))?;
        picked[q].push(o);
    }
    let mut text = answer
        .text
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty());
    let mut out = Vec::new();
    for (question, mut picks) in questions.iter().zip(picked) {
        picks.sort_unstable();
        picks.dedup();
        if picks.len() > 1 && !question.multi_select {
            return Err(bad(format!("{:?} takes one option", question.question)));
        }
        let value = if picks.is_empty() {
            text.take()
                .map(str::to_string)
                .ok_or_else(|| bad(format!("{:?} has no answer", question.question)))?
        } else {
            picks
                .iter()
                .map(|&o| question.options[o].label.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        out.push((question.question.clone(), value));
    }
    if text.is_some() {
        return Err(bad("the text answers no question".into()));
    }
    Ok(out)
}
