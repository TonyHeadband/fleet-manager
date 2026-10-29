//! Forms the daemon describes and every UI draws: which inputs, in what order, with which rules. Never where on
//! screen or how they move; each UI owns that. The daemon checks a request against the same rules with [`check`].
use crate::parse_date;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct FormDef {
    pub id: String,
    pub title: String,
    pub submit: SubmitDef,
    pub items: Vec<FormItem>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct SubmitDef {
    pub label: String,
    pub method: String,
    /// Path template; each `{name}` is filled in by the UI (e.g. `{product}`, `{id}`).
    pub path: String,
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum InputKind {
    Text,
    /// `YYYY-MM-DD`.
    Date,
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
            })
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

fn release_form(id: &str, title: &str, submit: SubmitDef) -> FormDef {
    let date = |key: &str, label: &str| InputDef {
        key: key.into(),
        label: label.into(),
        kind: InputKind::Date,
        placeholder: Some("YYYY-MM-DD".into()),
        default: None,
        rules: vec![Rule::Date],
    };
    FormDef {
        id: id.into(),
        title: title.into(),
        submit,
        items: vec![FormItem::Group {
            fields: vec![
                InputDef {
                    key: "version".into(),
                    label: "Version".into(),
                    kind: InputKind::Text,
                    placeholder: Some("1.0".into()),
                    default: None,
                    rules: vec![Rule::NotBlank {
                        message: Some("the release needs a version".into()),
                    }],
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
    }
}

pub fn release_create() -> FormDef {
    release_form(
        "release_create",
        "New release",
        SubmitDef {
            label: "Create".into(),
            method: "POST".into(),
            path: "/api/products/{product}/releases".into(),
        },
    )
}

pub fn release_edit() -> FormDef {
    release_form(
        "release_edit",
        "Edit release",
        SubmitDef {
            label: "Save".into(),
            method: "PATCH".into(),
            path: "/api/releases/{id}".into(),
        },
    )
}
