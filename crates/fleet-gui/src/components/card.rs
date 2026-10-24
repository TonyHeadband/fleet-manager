use super::Command;
use crate::theme::{palette, tone_color};
use crate::widgets::{button, field, ButtonVariant};
use eframe::egui::{self, Key, Margin, Sense, Stroke, TextEdit, Ui};
use fleet_api::{eval, ActionDef, Agent, Tone, UiSchema};
use fleet_client::schema::{column_for, visible_actions};
use std::collections::HashMap;

/// Per-card UI state that must survive between frames: answer boxes and a pending kill confirmation.
#[derive(Default)]
pub struct CardState {
    typed: HashMap<String, String>,
    confirming: Option<String>,
}

/// One agent's card: its schema fields, then its schema actions. Returns what the user asked for this frame.
pub fn card(ui: &mut Ui, schema: &UiSchema, agent: &Agent, now: i64, state: &mut CardState) -> Vec<Command> {
    let mut commands = Vec::new();
    let dark = ui.visuals().dark_mode;
    let p = palette(dark);
    let tone = column_for(schema, agent).map_or(Tone::Neutral, |c| c.tone);
    ui.push_id(&agent.id.0, |ui| {
        let frame = egui::Frame::new()
            .fill(p.surface)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(6)
            .inner_margin(Margin { left: 12, right: 8, top: 8, bottom: 8 });
        let shown = frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            // Labels must not swallow the click that opens the preview.
            ui.style_mut().interaction.selectable_labels = false;
            let body = ui
                .vertical(|ui| {
                    for f in schema.card_fields.iter().filter(|f| eval(&f.show_when, agent)) {
                        field(ui, f, agent, now);
                    }
                })
                .response;
            if ui.interact(body.rect, ui.scope_id().with("open"), Sense::click()).clicked() {
                commands.push(Command::Open(agent.id.0.clone()));
            }
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for action in visible_actions(schema, agent) {
                    control(ui, action, agent, state, &mut commands);
                }
            });
        });
        let r = shown.response.rect;
        let edge = egui::Rect::from_min_max(r.left_top(), egui::pos2(r.left() + 4.0, r.bottom()));
        ui.painter().rect_filled(edge, 2, tone_color(tone, dark));
    });
    commands
}

fn control(ui: &mut Ui, action: &ActionDef, agent: &Agent, state: &mut CardState, commands: &mut Vec<Command>) {
    let run = |input: Option<String>| Command::Run { action: action.clone(), agent: agent.clone(), input };
    if action.input.is_some() {
        let text = state.typed.entry(action.id.clone()).or_default();
        let edit = ui.add(TextEdit::singleline(text).id_salt(&action.id).hint_text(&action.label).desired_width(140.0));
        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let ready = !text.trim().is_empty();
        let clicked = ui.add_enabled_ui(ready, |ui| button(ui, &action.label, ButtonVariant::Default)).inner.clicked();
        if ready && (clicked || enter) {
            commands.push(run(Some(std::mem::take(text))));
        }
    } else if action.destructive {
        if state.confirming.as_deref() == Some(action.id.as_str()) {
            if button(ui, &format!("Confirm {}", action.label), ButtonVariant::Danger).clicked() {
                state.confirming = None;
                commands.push(run(None));
            }
        } else if button(ui, &action.label, ButtonVariant::QuietDanger).clicked() {
            state.confirming = Some(action.id.clone());
        }
    } else if button(ui, &action.label, ButtonVariant::Default).clicked() {
        commands.push(run(None));
    }
}
