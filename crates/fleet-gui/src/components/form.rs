//! A form the daemon describes (`fleet_api::FormDef`), drawn input by input. The caller owns the heading, the
//! buttons and the error line; this draws the inputs and keeps their values.
use crate::theme::{size, tokens};
use crate::views::releases::format_day;
use eframe::egui::{accesskit::Role, RichText, TextEdit, Ui, WidgetInfo};
use fleet_api::{DefaultValue, FormDef, FormItem, InputDef, InputKind, Rule};
use serde_json::Value;
use std::collections::BTreeMap;

/// Each input's text by its key.
pub type FormValues = BTreeMap<String, String>;

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

pub fn fields(ui: &mut Ui, form: &FormDef, values: &mut FormValues) {
    for item in &form.items {
        match item {
            FormItem::Field(i) => {
                ui.horizontal(|ui| input(ui, i, values));
            }
            FormItem::Group { fields } => {
                ui.horizontal(|ui| {
                    for i in fields {
                        input(ui, i, values);
                    }
                });
            }
            FormItem::Unknown => {}
        }
    }
}

fn input(ui: &mut Ui, def: &InputDef, values: &mut FormValues) {
    let p = tokens(ui.visuals().dark_mode);
    let width = match def.kind {
        InputKind::Date => 90.0,
        InputKind::Text | InputKind::Unknown => 110.0,
    };
    ui.label(
        RichText::new(&def.label)
            .size(size::SMALL)
            .color(p.text_low),
    );
    let text = values.entry(def.key.clone()).or_default();
    let mut edit = TextEdit::singleline(text).desired_width(width);
    if let Some(hint) = &def.placeholder {
        edit = edit.hint_text(hint);
    }
    ui.add(edit)
        .widget_info(|| WidgetInfo::labeled(Role::TextInput, true, &def.label));
}
