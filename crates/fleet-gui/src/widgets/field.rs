use super::badge::{badge, badge_accent};
use crate::theme::{palette, tone_color};
use eframe::egui::{RichText, Ui};
use fleet_api::{Agent, FieldDef, FieldKind, Tone};
use fleet_client::schema::{field_value, format_duration, FieldValue};

/// One schema-defined card field: the value is read from the agent and drawn by the field's kind.
/// The caller decides whether the field's `show_when` holds; empty, false and unknown values draw nothing.
pub fn field(ui: &mut Ui, def: &FieldDef, agent: &Agent, now: i64) {
    match field_value(agent, &def.key) {
        FieldValue::Absent => {}
        FieldValue::Flag => draw(ui, def, &def.label, now),
        FieldValue::Text(s) => draw(ui, def, &s, now),
        FieldValue::Number(n) => match def.kind {
            FieldKind::Duration => {
                let p = palette(ui.visuals().dark_mode);
                ui.label(RichText::new(format_duration(now - n)).small().color(p.muted)).on_hover_text(&def.label);
            }
            _ => draw(ui, def, &n.to_string(), now),
        },
        FieldValue::List(items) => {
            // A list field shows one element per entry.
            ui.horizontal_wrapped(|ui| {
                for item in items {
                    draw(ui, def, &item, now);
                }
            });
        }
    }
}

fn draw(ui: &mut Ui, def: &FieldDef, text: &str, _now: i64) {
    let dark = ui.visuals().dark_mode;
    let p = palette(dark);
    match def.kind {
        FieldKind::Badge => {
            // Mirrors the web stylesheet: the blocked-reason badge carries the attention colour.
            let accent = (def.key == "blocked_reason").then(|| tone_color(Tone::Attention, dark));
            if accent.is_some() {
                badge_accent(ui, text, &def.label, accent);
            } else {
                badge(ui, text, &def.label);
            }
        }
        FieldKind::Mono => {
            ui.label(RichText::new(text).monospace().small().color(p.muted)).on_hover_text(&def.label);
        }
        FieldKind::Duration | FieldKind::Text => {
            let rich = match def.key.as_str() {
                "task" => RichText::new(text).strong(),
                "activity" => RichText::new(text).small().color(p.muted),
                _ => RichText::new(text),
            };
            ui.label(rich);
        }
    }
}
