//! A form the daemon describes (`fleet_api::FormDef`), drawn input by input. The caller owns the heading, the
//! buttons and the error line; this draws the inputs and keeps their values.
use super::caption;
use crate::theme::{mono, size, tokens};
use crate::views::releases::format_day;
use crate::widgets::chip;
use eframe::egui::{self, accesskit::Role, RichText, TextEdit, Ui, WidgetInfo};
use fleet_api::{
    ChoiceSource, DefaultValue, FormDef, FormItem, InputDef, InputKind, RepoInfo, Rule,
};
use fleet_client::http::FleetError;
use serde_json::Value;
use std::collections::BTreeMap;

/// Each input's text by its key.
pub type FormValues = BTreeMap<String, String>;

/// What a `Choice` input offers, by its source: data the view already has for the record the form is about.
#[derive(Default, Clone, Copy)]
pub struct Sources<'a> {
    pub product_repos: &'a [RepoInfo],
}

impl Sources<'_> {
    fn options(&self, source: Option<&ChoiceSource>) -> Vec<&str> {
        match source {
            Some(ChoiceSource::ProductRepos) => {
                self.product_repos.iter().map(|r| r.name.as_str()).collect()
            }
            _ => vec![],
        }
    }
}

/// Defaults filled in; `today` is a local day number.
pub fn initial_values(form: &FormDef, today: i64) -> FormValues {
    form.inputs()
        .map(|i| {
            let value = match i.default {
                Some(DefaultValue::Today) => format_day(today),
                _ => String::new(),
            };
            (i.key.clone(), value)
        })
        .collect()
}

/// The values an existing record already has, for an edit.
pub fn values_from(form: &FormDef, record: &Value) -> FormValues {
    form.inputs()
        .map(|i| {
            let value = record.get(&i.key).and_then(Value::as_str).unwrap_or("");
            (i.key.clone(), value.to_string())
        })
        .collect()
}

/// Whether the form may be sent: only `NotBlank` is checked here, the daemon checks the rest.
pub fn ready(form: &FormDef, values: &FormValues) -> bool {
    form.inputs().all(|i| {
        let blank = values.get(&i.key).is_none_or(|v| v.trim().is_empty());
        !(blank && i.rules.iter().any(|r| matches!(r, Rule::NotBlank { .. })))
    })
}

/// The request body: every input's trimmed text under its key.
pub fn body(form: &FormDef, values: &FormValues) -> Value {
    form.inputs()
        .map(|i| {
            let v = values.get(&i.key).map_or("", |v| v.trim());
            (i.key.clone(), Value::from(v))
        })
        .collect::<serde_json::Map<_, _>>()
        .into()
}

/// `error` when it names one of the form's inputs: drawn under that input, so the caller draws only the others.
pub fn field_error<'a>(form: &FormDef, error: Option<&'a FleetError>) -> Option<&'a FleetError> {
    error.filter(|e| {
        e.field
            .as_deref()
            .is_some_and(|f| form.inputs().any(|i| i.key == f))
    })
}

/// The inputs; a refusal naming one of them is drawn under it.
pub fn fields(
    ui: &mut Ui,
    form: &FormDef,
    values: &mut FormValues,
    error: Option<&FleetError>,
    sources: &Sources,
) {
    let error = field_error(form, error);
    for item in &form.items {
        match item {
            FormItem::Field(i) => input(
                ui,
                i,
                values.entry(i.key.clone()).or_default(),
                error,
                sources,
            ),
            FormItem::Group { fields } => {
                ui.horizontal(|ui| {
                    for i in fields {
                        input(
                            ui,
                            i,
                            values.entry(i.key.clone()).or_default(),
                            error,
                            sources,
                        );
                    }
                });
            }
            FormItem::Unknown => {}
        }
    }
}

/// One input with its help line, and `error` under it when the refusal names it. For a view that keeps the value
/// itself rather than in `FormValues`.
pub fn input(
    ui: &mut Ui,
    def: &InputDef,
    value: &mut String,
    error: Option<&FleetError>,
    sources: &Sources,
) {
    let p = tokens(ui.visuals().dark_mode);
    let error = error.filter(|e| e.field.as_deref() == Some(def.key.as_str()));
    let small = |ui: &mut Ui, text: &str, color| {
        ui.add(egui::Label::new(RichText::new(text).size(size::SMALL).color(color)).wrap());
    };
    ui.vertical(|ui| {
        match def.kind {
            InputKind::Multiline | InputKind::Markdown => {
                let label = caption(ui, &def.label);
                if let Some(help) = &def.help {
                    ui.add_space(4.0);
                    small(ui, help, p.text_low);
                    ui.add_space(6.0);
                } else {
                    ui.add_space(4.0);
                }
                let mut edit = TextEdit::multiline(value).desired_width(f32::INFINITY);
                edit = if def.kind == InputKind::Markdown {
                    edit.font(mono(size::CODE)).desired_rows(24)
                } else {
                    edit.desired_rows(3)
                };
                if let Some(hint) = &def.placeholder {
                    edit = edit.hint_text(hint);
                }
                ui.add(edit).labelled_by(label.id);
            }
            InputKind::Choice => {
                caption(ui, &def.label);
                if let Some(help) = &def.help {
                    small(ui, help, p.text_low);
                }
                ui.add_space(2.0);
                ui.horizontal_wrapped(|ui| {
                    for option in sources.options(def.source.as_ref()) {
                        if chip(ui, option, value == option).clicked() {
                            *value = option.to_string();
                        }
                    }
                });
            }
            _ => {
                ui.horizontal(|ui| inline_edit(ui, def, value));
                if let Some(help) = &def.help {
                    small(ui, help, p.text_low);
                }
            }
        }
        if let Some(e) = error {
            small(ui, &e.message, p.danger);
        }
    });
}

fn inline_edit(ui: &mut Ui, def: &InputDef, value: &mut String) {
    let p = tokens(ui.visuals().dark_mode);
    let width = match def.kind {
        InputKind::Date => 90.0,
        _ => 110.0,
    };
    ui.label(
        RichText::new(&def.label)
            .size(size::SMALL)
            .color(p.text_low),
    );
    let mut edit = TextEdit::singleline(value).desired_width(width);
    if let Some(hint) = &def.placeholder {
        edit = edit.hint_text(hint);
    }
    ui.add(edit)
        .widget_info(|| WidgetInfo::labeled(Role::TextInput, true, &def.label));
}
