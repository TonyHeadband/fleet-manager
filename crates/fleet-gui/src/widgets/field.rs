use super::badge::{badge, BadgeTone};
use crate::theme::{mono, sans, size, tokens, Weight};
use eframe::egui::{Label, RichText, Ui};
use fleet_api::{Agent, FieldDef, FieldKind};

use super::format_bytes;
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
                let p = tokens(ui.visuals().dark_mode);
                ui.label(
                    RichText::new(format_duration(now - n))
                        .font(mono(size::META))
                        .color(p.text_low),
                )
                .on_hover_text(&def.label);
            }
            FieldKind::Bytes => {
                let p = tokens(ui.visuals().dark_mode);
                ui.label(
                    RichText::new(format_bytes(n.max(0) as u64))
                        .font(mono(size::META))
                        .color(p.text_low),
                )
                .on_hover_text(&def.label);
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
    let p = tokens(ui.visuals().dark_mode);
    match def.kind {
        // Repos sit in the card's meta row as plain mono labels; other badges are filled tags.
        FieldKind::Badge if def.key == "repos" => {
            ui.label(RichText::new(text).font(mono(size::META)).color(p.text_low));
        }
        FieldKind::Badge => {
            let (shown, tone) = badge_look(def, text);
            badge(ui, shown, &def.label, tone);
        }
        // One line with an ellipsis; an unbroken branch name must not widen the card.
        FieldKind::Mono => {
            let rich = RichText::new(text).font(mono(size::META)).color(p.text_low);
            ui.add(Label::new(rich).truncate().show_tooltip_when_elided(false))
                .on_hover_text(text);
        }
        FieldKind::Duration | FieldKind::Text | FieldKind::Bytes => match def.key.as_str() {
            // One line, like every card row; the side panel shows the whole title.
            "goal" => {
                let rich = RichText::new(text)
                    .font(sans(ui, size::TITLE, Weight::Regular))
                    .color(p.text_high);
                ui.add(Label::new(rich).truncate().show_tooltip_when_elided(false))
                    .on_hover_text(text);
            }
            // One line: a card has room for one reason line, and the hover shows the rest.
            "activity" => {
                let rich = RichText::new(text)
                    .font(sans(ui, size::SMALL, Weight::Regular))
                    .color(p.text_low);
                ui.add(Label::new(rich).truncate().show_tooltip_when_elided(false))
                    .on_hover_text(text);
            }
            _ => {
                ui.label(text);
            }
        },
    }
}

/// A badge's text and tone for one raw value. The daemon owns the wording and the tones, so a new raw value needs
/// no client change.
fn badge_look<'a>(def: &'a FieldDef, raw: &'a str) -> (&'a str, BadgeTone) {
    let shown = def.value_labels.get(raw).map_or(raw, String::as_str);
    let tone = match def.value_tones.get(raw) {
        Some(t) => BadgeTone::Tinted(*t),
        None if def.key == "blocked_reason" => BadgeTone::Attention,
        None => BadgeTone::Neutral,
    };
    (shown, tone)
}

/// The badges a badge field shows for `agent`, as `(text, tone)`; empty when it shows nothing. For a row that
/// has to know every badge before it draws one.
pub fn badge_items(def: &FieldDef, agent: &Agent) -> Vec<(String, BadgeTone)> {
    let raw: Vec<String> = match field_value(agent, &def.key) {
        FieldValue::Absent => vec![],
        FieldValue::Flag => vec![def.label.clone()],
        FieldValue::Text(s) => vec![s],
        FieldValue::Number(n) => vec![n.to_string()],
        FieldValue::List(items) => items,
    };
    raw.iter()
        .map(|r| {
            let (shown, tone) = badge_look(def, r);
            (shown.to_owned(), tone)
        })
        .collect()
}
