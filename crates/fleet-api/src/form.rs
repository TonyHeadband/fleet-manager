//! Forms the daemon describes and every UI draws: which inputs, in what order, with which rules. Never where on
//! screen or how they move; each UI owns that. The daemon checks a request against the same rules with [`check`].
use crate::parse_date;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use ts_rs::TS;
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct FormDef {
    pub id: String,
    pub title: String,
    pub submit: SubmitDef,
    pub items: Vec<FormItem>,
    /// Further actions after `submit` (Request changes, Refuse); a build that predates them never sees them.
    #[serde(default)]
    pub more: Vec<SubmitDef>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct SubmitDef {
    /// Names the action for [`check_submit`]; the daemon's route knows which one it is serving.
    #[serde(default)]
    pub id: String,
    pub label: String,
    pub method: String,
    /// Path template; each `{name}` is filled in by the UI (e.g. `{product}`, `{id}`).
    pub path: String,
    /// Sent as is with this action.
    #[serde(default)]
    pub fixed: BTreeMap<String, FixedValue>,
    /// Body keys the UI fills from what it shows (e.g. the breakdown version); left out when it has none.
    #[serde(default)]
    pub context: Vec<String>,
    /// Inputs that must not be blank for this action alone.
    #[serde(default)]
    pub needs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[serde(untagged)]
#[ts(export)]
pub enum FixedValue {
    Bool(bool),
    Text(String),
}

/// An unknown kind, item or rule (from a newer daemon) parses as `Unknown`, so the rest of `UiSchema` still loads;
/// a UI does not draw a form holding one and uses its own copy instead.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum FormItem {
    Field(InputDef),
    /// Inputs that belong together; each UI decides how to show that (egui: one line).
    Group {
        fields: Vec<InputDef>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct InputDef {
    /// The request-body key the value is sent under.
    pub key: String,
    pub label: String,
    pub kind: InputKind,
    #[serde(default)]
    pub placeholder: Option<String>,
    #[serde(default)]
    pub default: Option<DefaultValue>,
    #[serde(default)]
    pub rules: Vec<Rule>,
    /// A line under the label saying what the input is for.
    #[serde(default)]
    pub help: Option<String>,
    /// Where a `Choice` takes its options from.
    #[serde(default)]
    pub source: Option<ChoiceSource>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum InputKind {
    Text,
    /// `YYYY-MM-DD`.
    Date,
    Multiline,
    /// Multiline, in the mono face.
    Markdown,
    /// One of `InputDef.source`'s options.
    Choice,
    #[serde(other)]
    Unknown,
}

/// Data every UI already has for the record a form is about.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum ChoiceSource {
    /// The repos of the task's product, by name.
    ProductRepos,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum DefaultValue {
    /// The user's local date.
    Today,
    #[serde(other)]
    Unknown,
}

/// A rule applies only when its key is in the request body, so a partial update is checked on what it sends.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum Rule {
    /// The only rule a UI checks before sending; the rest come back from the daemon with the field named.
    NotBlank {
        #[serde(default)]
        message: Option<String>,
    },
    Date,
    /// On or after the date in `field`.
    NotBefore {
        field: String,
        #[serde(default)]
        message: Option<String>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

impl FormDef {
    /// Every input in order, groups flattened.
    pub fn inputs(&self) -> impl Iterator<Item = &InputDef> {
        self.items.iter().flat_map(|item| match item {
            FormItem::Field(i) => std::slice::from_ref(i),
            FormItem::Group { fields } => fields.as_slice(),
            FormItem::Unknown => &[],
        })
    }

    pub fn is_known(&self) -> bool {
        self.items.iter().all(|i| !matches!(i, FormItem::Unknown))
            && self.inputs().all(|i| {
                i.kind != InputKind::Unknown
                    && i.default != Some(DefaultValue::Unknown)
                    && !i.rules.contains(&Rule::Unknown)
                    && match (&i.kind, &i.source) {
                        (InputKind::Choice, Some(source)) => *source != ChoiceSource::Unknown,
                        (InputKind::Choice, None) => false,
                        _ => true,
                    }
            })
    }

    /// `submit`, then `more`.
    pub fn submits(&self) -> impl Iterator<Item = &SubmitDef> {
        std::iter::once(&self.submit).chain(&self.more)
    }

    pub fn submit_by_id(&self, id: &str) -> Option<&SubmitDef> {
        self.submits().find(|s| s.id == id)
    }
}

/// The first rule `body` breaks, in input order.
pub fn check(form: &FormDef, body: &Value) -> Result<(), FieldError> {
    let text = |key: &str| body.get(key).and_then(Value::as_str).map(str::trim);
    for input in form.inputs() {
        let Some(value) = text(&input.key) else {
            continue;
        };
        for rule in &input.rules {
            let message = match rule {
                Rule::NotBlank { message } if value.is_empty() => message
                    .clone()
                    .unwrap_or_else(|| format!("{} is required", input.label)),
                Rule::Date if parse_date(value).is_none() => {
                    format!("{value:?} is not a date; use YYYY-MM-DD")
                }
                Rule::NotBefore { field, message } => {
                    let before = text(field)
                        .and_then(parse_date)
                        .zip(parse_date(value))
                        .is_some_and(|(other, this)| this < other);
                    if !before {
                        continue;
                    }
                    message.clone().unwrap_or_else(|| {
                        let other = form
                            .inputs()
                            .find(|i| &i.key == field)
                            .map_or(field.as_str(), |i| i.label.as_str());
                        format!("{} is before {other}", input.label)
                    })
                }
                _ => continue,
            };
            return Err(FieldError {
                field: input.key.clone(),
                message,
            });
        }
    }
    Ok(())
}

/// [`check`], then the inputs action `id` needs. An id the form does not have checks the inputs only.
pub fn check_submit(form: &FormDef, id: &str, body: &Value) -> Result<(), FieldError> {
    check(form, body)?;
    let Some(submit) = form.submit_by_id(id) else {
        return Ok(());
    };
    for key in &submit.needs {
        let blank = body
            .get(key)
            .and_then(Value::as_str)
            .is_none_or(|v| v.trim().is_empty());
        if !blank {
            continue;
        }
        let input = form.inputs().find(|i| &i.key == key);
        let message = input
            .and_then(|i| {
                i.rules.iter().find_map(|r| match r {
                    Rule::NotBlank { message } => message.clone(),
                    _ => None,
                })
            })
            .unwrap_or_else(|| format!("{} is required", input.map_or(key.as_str(), |i| &i.label)));
        return Err(FieldError {
            field: key.clone(),
            message,
        });
    }
    Ok(())
}

fn input(key: &str, label: &str, kind: InputKind) -> InputDef {
    InputDef {
        key: key.into(),
        label: label.into(),
        kind,
        placeholder: None,
        default: None,
        rules: vec![],
        help: None,
        source: None,
    }
}

fn action(id: &str, label: &str, method: &str, path: &str) -> SubmitDef {
    SubmitDef {
        id: id.into(),
        label: label.into(),
        method: method.into(),
        path: path.into(),
        ..SubmitDef::default()
    }
}

fn form(id: &str, title: &str, submit: SubmitDef, items: Vec<FormItem>) -> FormDef {
    FormDef {
        id: id.into(),
        title: title.into(),
        submit,
        items,
        more: vec![],
    }
}

fn release_form(id: &str, title: &str, submit: SubmitDef) -> FormDef {
    let date = |key: &str, label: &str| InputDef {
        placeholder: Some("YYYY-MM-DD".into()),
        rules: vec![Rule::Date],
        ..input(key, label, InputKind::Date)
    };
    form(
        id,
        title,
        submit,
        vec![FormItem::Group {
            fields: vec![
                InputDef {
                    placeholder: Some("1.0".into()),
                    rules: vec![Rule::NotBlank {
                        message: Some("the release needs a version".into()),
                    }],
                    ..input("version", "Version", InputKind::Text)
                },
                InputDef {
                    default: Some(DefaultValue::Today),
                    ..date("start", "Start")
                },
                InputDef {
                    rules: vec![
                        Rule::Date,
                        Rule::NotBefore {
                            field: "start".into(),
                            message: Some("the target date is before the start date".into()),
                        },
                    ],
                    ..date("target", "Target")
                },
            ],
        }],
    )
}

pub fn release_create() -> FormDef {
    release_form(
        "release_create",
        "New release",
        action(
            "create",
            "Create",
            "POST",
            "/api/products/{product}/releases",
        ),
    )
}

pub fn release_edit() -> FormDef {
    release_form(
        "release_edit",
        "Edit release",
        action("save", "Save", "PATCH", "/api/releases/{id}"),
    )
}

pub fn instructions() -> FormDef {
    form(
        "instructions",
        "Instructions for every agent",
        action("save", "Save", "PUT", "/api/agent-config"),
        vec![FormItem::Field(InputDef {
            help: Some(
                "Markdown, put at the top of each agent's CLAUDE.md under \"# Instructions for every agent\"."
                    .into(),
            ),
            ..input(
                "instructions",
                "Instructions for every agent",
                InputKind::Markdown,
            )
        })],
    )
}

pub fn breakdown_decision() -> FormDef {
    let path = "/api/tasks/{id}/breakdown/decide";
    let decide = |id: &str, label: &str, approve: bool| SubmitDef {
        fixed: [("approve".to_string(), FixedValue::Bool(approve))].into(),
        context: vec!["version".into()],
        ..action(id, label, "POST", path)
    };
    FormDef {
        more: vec![SubmitDef {
            needs: vec!["comment".into()],
            ..decide("request_changes", "Request changes", false)
        }],
        ..form(
            "breakdown_decision",
            "Breakdown",
            decide("approve", "Approve", true),
            vec![FormItem::Field(InputDef {
                placeholder: Some("What should change?".into()),
                ..input("comment", "Or ask for changes", InputKind::Multiline)
            })],
        )
    }
}

pub fn repo_request() -> FormDef {
    let local = |s: SubmitDef| SubmitDef {
        fixed: [("actor".to_string(), FixedValue::Text("local".into()))].into(),
        ..s
    };
    FormDef {
        more: vec![local(action(
            "refuse",
            "Refuse",
            "POST",
            "/api/agents/{id}/repo-request/refuse",
        ))],
        ..form(
            "repo_request",
            "Repo request",
            SubmitDef {
                needs: vec!["repo".into()],
                ..local(action(
                    "grant",
                    "Grant",
                    "POST",
                    "/api/agents/{id}/repo-request/grant",
                ))
            },
            vec![
                FormItem::Field(InputDef {
                    source: Some(ChoiceSource::ProductRepos),
                    ..input("repo", "Grant access to", InputKind::Choice)
                }),
                FormItem::Field(input("reason", "Reason (optional)", InputKind::Text)),
            ],
        )
    }
}
