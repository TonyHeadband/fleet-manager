//! What the daemon's `UiSchema` means for one agent. Ports of `web/src/api/schema.ts`; the
//! condition language itself is `fleet_api::eval`, shared with the daemon.
use fleet_api::{eval, ActionDef, Agent, ColumnDef, UiSchema};
use serde_json::{json, Value};

pub fn column_for<'a>(schema: &'a UiSchema, a: &Agent) -> Option<&'a ColumnDef> {
    schema
        .columns
        .iter()
        .find(|c| c.statuses.contains(&a.status))
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
