use crate::{
    breakdown_decision, instructions, release_create, release_edit, repo_request, Agent,
    BlockedReason, FormDef, Status,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;
use utoipa::ToSchema;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Tone {
    Neutral,
    Active,
    Attention,
    Done,
    Muted,
    /// Amber: a deliberate stop, nothing went wrong.
    Caution,
    /// Red: something failed (a crash).
    Danger,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FieldKind {
    Text,
    Duration,
    Badge,
    Mono,
    /// A size in bytes, shown in SI units ("7.6 GB").
    Bytes,
}

/// A predicate over an agent, evaluated by both daemon and clients with [`eval`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum Condition {
    Always,
    StatusIn {
        statuses: Vec<Status>,
    },
    Paused {
        value: bool,
    },
    HasChanges {
        value: bool,
    },
    BlockedReasonIn {
        reasons: Vec<BlockedReason>,
    },
    Not {
        #[schema(no_recursion)]
        condition: Box<Condition>,
    },
    All {
        #[schema(no_recursion)]
        conditions: Vec<Condition>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct ColumnDef {
    pub id: String,
    pub title: String,
    pub statuses: Vec<Status>,
    pub empty_hint: String,
    pub tone: Tone,
    /// Drawn on a darker background: the column holds what is no longer running.
    #[serde(default)]
    pub dim: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct FieldDef {
    /// Name of the `Agent` field to render.
    pub key: String,
    pub label: String,
    pub kind: FieldKind,
    pub show_when: Condition,
    /// Display text per raw value, for badge fields (e.g. `repo_access` -> "Repo access").
    #[serde(default)]
    pub value_labels: BTreeMap<String, String>,
    /// A badge's tone per raw value (e.g. `stop_cause` "crashed" -> danger); a value with none is neutral.
    #[serde(default)]
    pub value_tones: BTreeMap<String, Tone>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct ActionDef {
    pub id: String,
    pub label: String,
    pub method: String,
    /// Path template; `{id}` is replaced with the agent id.
    pub path: String,
    pub when: Condition,
    /// Request-body field the UI must collect from the user, e.g. `"text"`.
    pub input: Option<String>,
    pub destructive: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct UiSchema {
    pub columns: Vec<ColumnDef>,
    pub card_fields: Vec<FieldDef>,
    pub actions: Vec<ActionDef>,
    #[serde(default)]
    pub forms: Vec<FormDef>,
}

impl UiSchema {
    pub fn form(&self, id: &str) -> Option<&FormDef> {
        self.forms.iter().find(|f| f.id == id)
    }
}

pub fn eval(c: &Condition, a: &Agent) -> bool {
    match c {
        Condition::Always => true,
        Condition::StatusIn { statuses } => statuses.contains(&a.status),
        Condition::Paused { value } => a.paused == *value,
        Condition::HasChanges { value } => a.has_changes == *value,
        Condition::BlockedReasonIn { reasons } => {
            a.blocked_reason.is_some_and(|r| reasons.contains(&r))
        }
        Condition::Not { condition } => !eval(condition, a),
        Condition::All { conditions } => conditions.iter().all(|c| eval(c, a)),
    }
}

fn column(id: &str, title: &str, statuses: &[Status], hint: &str, tone: Tone) -> ColumnDef {
    ColumnDef {
        id: id.into(),
        title: title.into(),
        statuses: statuses.to_vec(),
        empty_hint: hint.into(),
        tone,
        dim: false,
    }
}

fn field(key: &str, label: &str, kind: FieldKind, show_when: Condition) -> FieldDef {
    FieldDef {
        key: key.into(),
        label: label.into(),
        kind,
        show_when,
        value_labels: BTreeMap::new(),
        value_tones: BTreeMap::new(),
    }
}

fn labels(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn action(id: &str, label: &str, method: &str, path: &str, when: Condition) -> ActionDef {
    ActionDef {
        id: id.into(),
        label: label.into(),
        method: method.into(),
        path: path.into(),
        when,
        input: None,
        destructive: false,
    }
}

pub fn default_ui_schema() -> UiSchema {
    use Status::*;
    UiSchema {
        columns: vec![
            ColumnDef {
                dim: true,
                ..column(
                    "stopped",
                    "Stopped",
                    &[Stopped],
                    "Nothing stopped",
                    Tone::Muted,
                )
            },
            column(
                "queued",
                "Queued",
                &[Queued],
                "Nothing waiting to start",
                Tone::Neutral,
            ),
            column(
                "working",
                "Working",
                &[Working],
                "No agent is working",
                Tone::Active,
            ),
            column(
                "needs_you",
                "Needs you",
                &[NeedsYou],
                "No agent needs you",
                Tone::Attention,
            ),
            // No Talking column: nothing sets that status yet (agent-to-agent messaging, #44).
            column(
                "review",
                "Review",
                &[Review],
                "Nothing to review",
                Tone::Done,
            ),
        ],
        card_fields: vec![
            // Derived by the client: one badge per workspace repo.
            field("repos", "Repos", FieldKind::Badge, Condition::Always),
            field("title", "Task", FieldKind::Text, Condition::Always),
            field("activity", "Activity", FieldKind::Text, Condition::Always),
            field("branch", "Branch", FieldKind::Mono, Condition::Always),
            field(
                "status_since",
                "In status",
                FieldKind::Duration,
                Condition::Always,
            ),
            FieldDef {
                value_labels: labels(&[
                    ("permission", "Permission needed"),
                    ("question", "Question"),
                    ("error", "Error"),
                    ("startup", "Startup prompt"),
                    ("repo_access", "Repo access"),
                    ("breakdown", "Breakdown to review"),
                ]),
                ..field(
                    "blocked_reason",
                    "Blocked",
                    FieldKind::Badge,
                    Condition::StatusIn {
                        statuses: vec![NeedsYou],
                    },
                )
            },
            field(
                "has_changes",
                "Has changes",
                FieldKind::Badge,
                Condition::HasChanges { value: true },
            ),
            field(
                "paused",
                "Paused",
                FieldKind::Badge,
                Condition::Paused { value: true },
            ),
            FieldDef {
                value_labels: labels(&[
                    ("stopped", "Stopped"),
                    ("exited", "Exited"),
                    ("crashed", "Crashed"),
                    ("finished", "Finished"),
                    ("interrupted", "Interrupted"),
                ]),
                value_tones: [
                    ("stopped", Tone::Caution),
                    ("exited", Tone::Active),
                    ("crashed", Tone::Danger),
                    ("finished", Tone::Active),
                    ("interrupted", Tone::Caution),
                ]
                .into_iter()
                .map(|(k, t)| (k.to_string(), t))
                .collect(),
                ..field(
                    "stop_cause",
                    "Why it stopped",
                    FieldKind::Badge,
                    Condition::StatusIn {
                        statuses: vec![Stopped],
                    },
                )
            },
            field(
                "disk_bytes",
                "Size on disk",
                FieldKind::Bytes,
                Condition::StatusIn {
                    statuses: vec![Stopped],
                },
            ),
        ],
        actions: vec![
            ActionDef {
                input: Some("text".into()),
                // A repo-access request is granted or refused, not answered with text.
                ..action(
                    "answer",
                    "Answer",
                    "POST",
                    "/api/agents/{id}/answer",
                    Condition::All {
                        conditions: vec![
                            Condition::StatusIn {
                                statuses: vec![Working, NeedsYou, Review],
                            },
                            Condition::Not {
                                condition: Box::new(Condition::BlockedReasonIn {
                                    reasons: vec![BlockedReason::RepoAccess],
                                }),
                            },
                        ],
                    },
                )
            },
            action(
                "pause",
                "Pause",
                "POST",
                "/api/agents/{id}/pause",
                // Not on Needs you: Escape there dismisses the permission request as "No".
                Condition::All {
                    conditions: vec![
                        Condition::Paused { value: false },
                        Condition::StatusIn {
                            statuses: vec![Working, Review],
                        },
                    ],
                },
            ),
            action(
                "resume",
                "Resume",
                "POST",
                "/api/agents/{id}/resume",
                Condition::All {
                    conditions: vec![
                        Condition::Paused { value: true },
                        Condition::StatusIn {
                            statuses: vec![Queued, Working, NeedsYou, Review],
                        },
                    ],
                },
            ),
            ActionDef {
                destructive: true,
                ..action(
                    "stop",
                    "Stop",
                    "POST",
                    "/api/agents/{id}/stop",
                    Condition::Not {
                        condition: Box::new(Condition::StatusIn {
                            statuses: vec![Stopped],
                        }),
                    },
                )
            },
        ],
        forms: vec![
            release_create(),
            release_edit(),
            instructions(),
            breakdown_decision(),
            repo_request(),
        ],
    }
}
