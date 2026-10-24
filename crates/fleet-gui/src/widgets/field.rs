use super::badge::{badge, BadgeTone};
use crate::theme::{mono, sans, size, tokens, Weight};
use eframe::egui::text::{LayoutJob, TextFormat, TextWrapping};
use eframe::egui::{Label, RichText, TextWrapMode, Ui};
use fleet_api::{Agent, FieldDef, FieldKind};
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
                ui.label(RichText::new(format_duration(now - n)).font(mono(size::META)).color(p.text_low)).on_hover_text(&def.label);
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
            let tone = if def.key == "blocked_reason" { BadgeTone::Attention } else { BadgeTone::Neutral };
            // The daemon owns the wording, so a new raw value needs no client change.
            let shown = def.value_labels.get(text).map_or(text, String::as_str);
            badge(ui, shown, &def.label, tone);
        }
        // One line with an ellipsis; an unbroken branch name must not widen the card.
        FieldKind::Mono => {
            let rich = RichText::new(text).font(mono(size::META)).color(p.text_low);
            ui.add(Label::new(rich).truncate().show_tooltip_when_elided(false)).on_hover_text(text);
        }
        FieldKind::Duration | FieldKind::Text => match def.key.as_str() {
            "task" => {
                // Label would overwrite a wrap set on a RichText, so the limits go on a LayoutJob.
                let format = TextFormat {
                    font_id: sans(ui, size::TITLE, Weight::Regular),
                    color: p.text_high,
                    line_height: Some(size::TITLE * 1.35),
                    ..Default::default()
                };
                let mut job = LayoutJob::single_section(text.to_owned(), format);
                // Breaks between words; egui still splits a single word wider than the card.
                job.wrap = TextWrapping { max_rows: 2, break_anywhere: false, overflow_character: Some('…'), ..Default::default() };
                ui.add(Label::new(job).wrap_mode(TextWrapMode::Wrap).show_tooltip_when_elided(false)).on_hover_text(text);
            }
            "activity" => {
                ui.label(RichText::new(text).font(sans(ui, size::SMALL, Weight::Regular)).color(p.text_low));
            }
            _ => {
                ui.label(text);
            }
        },
    }
}
