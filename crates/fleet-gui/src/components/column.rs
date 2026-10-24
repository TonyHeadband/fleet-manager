use super::card::{card, CardState};
use super::Command;
use crate::theme::{palette, tone_color};
use eframe::egui::{self, RichText, ScrollArea, Stroke, Ui};
use fleet_api::{Agent, ColumnDef, UiSchema};
use std::collections::HashMap;

/// One column: title, count and its cards, longest-waiting first, scrolling on its own.
pub fn column(
    ui: &mut Ui,
    def: &ColumnDef,
    schema: &UiSchema,
    agents: &[&Agent],
    now: i64,
    states: &mut HashMap<String, CardState>,
) -> Vec<Command> {
    let dark = ui.visuals().dark_mode;
    let p = palette(dark);
    let mut cards: Vec<&Agent> = agents.to_vec();
    cards.sort_by_key(|a| a.status_since);
    let mut commands = Vec::new();
    let shown = egui::Frame::new().stroke(Stroke::new(1.0, p.border)).corner_radius(8).inner_margin(8).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(&def.title).strong());
            ui.label(RichText::new(cards.len().to_string()).color(p.muted));
        });
        ui.add_space(4.0);
        ScrollArea::vertical().id_salt(&def.id).auto_shrink([false, false]).show(ui, |ui| {
            if cards.is_empty() {
                ui.label(RichText::new(&def.empty_hint).small().color(p.muted));
            }
            for a in cards {
                commands.extend(card(ui, schema, a, now, states.entry(a.id.0.clone()).or_default()));
                ui.add_space(6.0);
            }
        });
    });
    let r = shown.response.rect;
    ui.painter().line_segment([r.left_top(), r.right_top()], Stroke::new(3.0, tone_color(def.tone, dark)));
    commands
}
