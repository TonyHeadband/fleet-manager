//! What the daemon's `UiSchema` means for one agent. Ports of `web/src/api/schema.ts`; the
//! condition language itself is `fleet_api::eval`, shared with the daemon.
use fleet_api::{
    eval, project_id, rollup, ActionDef, Agent, AgentId, AgentKind, ColumnDef, Status, UiSchema,
};
use serde_json::{json, Value};

pub fn column_for<'a>(schema: &'a UiSchema, a: &Agent) -> Option<&'a ColumnDef> {
    column_for_status(schema, a.status)
}

pub fn column_for_status(schema: &UiSchema, status: Status) -> Option<&ColumnDef> {
    schema.columns.iter().find(|c| c.statuses.contains(&status))
}

/// One entry on the board: a solo agent alone, or an orchestrated project's lead (its live orchestrator) with
/// every other agent of the project as its fleet. `status` is the column the whole group sits in.
#[derive(Debug, Clone, PartialEq)]
pub struct Group<'a> {
    pub lead: &'a Agent,
    /// Stuck agents first, then longest in their status first.
    pub fleet: Vec<&'a Agent>,
    pub status: Status,
}

impl<'a> Group<'a> {
    /// The orchestrator is gone while fleet agents still run: the project waits for a restart.
    pub fn lead_down(&self) -> bool {
        self.lead.status == Status::Stopped && self.live_fleet().next().is_some()
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

    /// An orchestrated project, drawn as a project card even before its first fleet agent exists.
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
    let mut projects: Vec<(&AgentId, Vec<&Agent>)> = Vec::new();
    for a in agents {
        let id = project_id(a);
        match projects.iter_mut().find(|(p, _)| *p == id) {
            Some((_, members)) => members.push(a),
            None => projects.push((id, vec![a])),
        }
    }
    projects
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
            let live = fleet.iter().filter(|a| a.status != Status::Stopped);
            // A crashed orchestrator leaves its fleet unsupervised: the project needs you, whatever they do.
            let status = if lead.status == Status::Stopped && live.clone().next().is_some() {
                Status::NeedsYou
            } else {
                rollup(std::iter::once(lead.status).chain(live.map(|a| a.status)))
            };
            Group {
                lead,
                fleet,
                status,
            }
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

/// The value a card field shows. Most keys are plain `Agent` fields; `repos` and `branch` are derived from
/// the workspaces, the only place a client knows how a schema key maps to the agent.
pub fn field_value(a: &Agent, key: &str) -> FieldValue {
    match key {
        "repos" => FieldValue::List(a.workspaces.iter().map(|w| w.repo.clone()).collect()),
        "branch" => {
            // Every repo of a project sits on the same branch name, so show it once.
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
