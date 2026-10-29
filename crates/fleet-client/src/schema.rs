//! What the daemon's `UiSchema` means for one agent. Ports of `web/src/api/schema.ts`; the
//! condition language itself is `fleet_api::eval`, shared with the daemon.
use fleet_api::{
    default_ui_schema, eval, rollup, task_id, ActionDef, Agent, AgentId, AgentKind, ColumnDef,
    FormDef, Status, StopCause, UiSchema,
};
use serde_json::{json, Value};

/// The daemon's copy of form `id` when this build understands all of it, else the built-in one: a daemon newer
/// (or older) than this client still gets a working form.
pub fn usable_form(schema: Option<&UiSchema>, id: &str) -> Option<FormDef> {
    schema
        .and_then(|s| s.form(id))
        .filter(|f| f.is_known())
        .cloned()
        .or_else(|| default_ui_schema().form(id).cloned())
}

pub fn column_for<'a>(schema: &'a UiSchema, a: &Agent) -> Option<&'a ColumnDef> {
    column_for_status(schema, a.status)
}

pub fn column_for_status(schema: &UiSchema, status: Status) -> Option<&ColumnDef> {
    schema.columns.iter().find(|c| c.statuses.contains(&status))
}

/// One entry on the board: a solo agent alone, or an orchestrated task's lead (its live orchestrator) with
/// every other agent of the task as its fleet. `status` is the column the whole group sits in.
#[derive(Debug, Clone, PartialEq)]
pub struct Group<'a> {
    pub lead: &'a Agent,
    /// Stuck agents first, then longest in their status first.
    pub fleet: Vec<&'a Agent>,
    pub status: Status,
}

impl<'a> Group<'a> {
    /// The orchestrator is gone and the task waits for a restart: its fleet still runs, or it crashed or exited
    /// on its own. The board has no breakdown to tell a finished task by, but a finished orchestrator opens its
    /// PR and sits in Review, so one that ended by itself left work behind. One stopped by its owner did not.
    pub fn lead_down(&self) -> bool {
        self.lead.status == Status::Stopped
            && (self.live_fleet().next().is_some()
                || (self.lead.kind == AgentKind::Orchestrator
                    && matches!(
                        self.lead.stop_cause,
                        Some(StopCause::Crashed | StopCause::Exited)
                    )))
    }

    /// Restart orchestrator is offered whenever the task's orchestrator has ended, whatever ended it.
    pub fn can_restart(&self) -> bool {
        self.lead.kind == AgentKind::Orchestrator && self.lead.status == Status::Stopped
    }

    /// The fleet agent the group waits on you for, when the lead itself does not.
    pub fn stuck(&self) -> Option<&'a Agent> {
        if self.lead.status == Status::NeedsYou {
            return None;
        }
        self.fleet
            .iter()
            .copied()
            .find(|a| a.status == Status::NeedsYou)
    }

    /// An orchestrated task, drawn as a task card even before its first fleet agent exists.
    pub fn orchestrated(&self) -> bool {
        self.lead.kind == AgentKind::Orchestrator || !self.fleet.is_empty()
    }

    /// How long the group has been in its column, for the column's longest-waiting-first order: the earliest
    /// `status_since` among the agents that put it there.
    pub fn since(&self) -> i64 {
        std::iter::once(self.lead)
            .chain(self.live_fleet())
            .filter(|a| rollup([a.status]) == self.status)
            .map(|a| a.status_since)
            .min()
            .unwrap_or(self.lead.status_since)
    }

    fn live_fleet(&self) -> impl Iterator<Item = &'a Agent> + '_ {
        self.fleet
            .iter()
            .copied()
            .filter(|a| a.status != Status::Stopped)
    }
}

/// The board's entries, in the order their first agent appears in `agents`.
pub fn groups(agents: &[Agent]) -> Vec<Group<'_>> {
    let mut tasks: Vec<(&AgentId, Vec<&Agent>)> = Vec::new();
    for a in agents {
        let id = task_id(a);
        match tasks.iter_mut().find(|(p, _)| *p == id) {
            Some((_, members)) => members.push(a),
            None => tasks.push((id, vec![a])),
        }
    }
    tasks
        .into_iter()
        .map(|(_, members)| {
            // `agents` comes in creation order, so the last orchestrator is the newest; a live one wins over a
            // stopped one whatever the order, since a restart only happens once the old one is gone.
            let lead_at = members
                .iter()
                .enumerate()
                .filter(|(_, a)| a.kind == AgentKind::Orchestrator)
                .max_by_key(|(i, a)| (a.status != Status::Stopped, *i))
                .map_or(0, |(i, _)| i);
            let lead = members[lead_at];
            let mut fleet: Vec<&Agent> = members
                .into_iter()
                .enumerate()
                .filter(|(i, _)| *i != lead_at)
                .map(|(_, a)| a)
                .collect();
            fleet.sort_by_key(|a| (a.status != Status::NeedsYou, a.status_since));
            let mut group = Group {
                lead,
                fleet,
                status: Status::Stopped,
            };
            // An orchestrator that is down leaves its task unsupervised: it needs you, whatever the fleet does.
            group.status = if group.lead_down() {
                Status::NeedsYou
            } else {
                rollup(std::iter::once(lead.status).chain(group.live_fleet().map(|a| a.status)))
            };
            group
        })
        .collect()
}

pub fn visible_actions<'a>(schema: &'a UiSchema, a: &Agent) -> Vec<&'a ActionDef> {
    schema
        .actions
        .iter()
        .filter(|act| eval(&act.when, a))
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActionRequest {
    pub method: String,
    pub path: String,
    pub body: Option<Value>,
}

/// Percent-encodes everything outside RFC 3986's unreserved set, so an id can never alter the path.
pub(crate) fn encode_path_segment(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub fn action_request(action: &ActionDef, agent: &Agent, input: Option<&str>) -> ActionRequest {
    let path = action
        .path
        .replace("{id}", &encode_path_segment(&agent.id.0));
    let method = action.method.clone();
    if method == "GET" || method == "DELETE" {
        return ActionRequest {
            method,
            path,
            body: None,
        };
    }
    let mut body = json!({ "actor": "local" });
    if let (Some(name), Some(text)) = (&action.input, input) {
        body[name.as_str()] = Value::String(text.to_string());
    }
    ActionRequest {
        method,
        path,
        body: Some(body),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FieldValue {
    /// Nothing to show: missing, null, empty text or `false`.
    Absent,
    /// A `true` boolean; drawn as the field's label.
    Flag,
    Text(String),
    Number(i64),
    /// One entry per element (the derived `repos` and `branch`).
    List(Vec<String>),
}

/// The longest title a goal is cut to, in characters, before the "…".
const TITLE_CHARS: usize = 60;

/// What the agent is called on its card and wherever it is named: its title, or a cut of its goal until it has one.
pub fn title(a: &Agent) -> String {
    a.title.clone().unwrap_or_else(|| short_title(&a.goal))
}

/// The goal's first sentence, cut at a word to `TITLE_CHARS` with "…". A dot only ends a sentence before a space,
/// so a version or a file name stays whole; a question keeps its "?".
pub fn short_title(goal: &str) -> String {
    let goal = goal.trim();
    let end = goal
        .char_indices()
        .find(|&(i, c)| {
            matches!(c, '.' | '?' | '!') && goal[i + 1..].starts_with(char::is_whitespace)
        })
        .map_or(goal.len(), |(i, c)| if c == '.' { i } else { i + 1 });
    let sentence = &goal[..end];
    if sentence.chars().count() <= TITLE_CHARS {
        return sentence.to_string();
    }
    let cut: String = sentence.chars().take(TITLE_CHARS).collect();
    let cut = match cut.rfind(char::is_whitespace) {
        Some(i) if i > 0 => cut[..i].trim_end(),
        _ => cut.as_str(),
    };
    format!("{cut}…")
}

/// The value a card field shows. Most keys are plain `Agent` fields; `repos` and `branch` are derived from
/// the workspaces and `title` falls back to the goal, the only place a client knows how a schema key maps to the
/// agent.
pub fn field_value(a: &Agent, key: &str) -> FieldValue {
    match key {
        "title" => match title(a) {
            t if t.is_empty() => FieldValue::Absent,
            t => FieldValue::Text(t),
        },
        "repos" => FieldValue::List(a.workspaces.iter().map(|w| w.repo.clone()).collect()),
        "branch" => {
            // Every repo of a task sits on the same branch name, so show it once.
            let mut seen: Vec<String> = Vec::new();
            for w in &a.workspaces {
                if !seen.contains(&w.branch) {
                    seen.push(w.branch.clone());
                }
            }
            FieldValue::List(seen)
        }
        _ => match serde_json::to_value(a)
            .ok()
            .and_then(|v| v.get(key).cloned())
        {
            Some(Value::String(s)) if !s.is_empty() => FieldValue::Text(s),
            Some(Value::Number(n)) => n.as_i64().map_or(FieldValue::Absent, FieldValue::Number),
            Some(Value::Bool(true)) => FieldValue::Flag,
            _ => FieldValue::Absent,
        },
    }
}

pub fn format_duration(seconds: i64) -> String {
    let s = seconds.max(0);
    if s < 60 {
        return format!("{s}s");
    }
    let m = s / 60;
    if m < 60 {
        return format!("{m}m");
    }
    let h = m / 60;
    if h < 24 {
        return format!("{h}h {}m", m % 60);
    }
    format!("{}d {}h", h / 24, h % 24)
}
