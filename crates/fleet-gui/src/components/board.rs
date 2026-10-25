use super::card::CardState;
use super::column::column;
use super::Command;
use crate::theme::tokens;
use eframe::egui::{Align, Layout, Stroke, Ui};
use fleet_api::{Agent, UiSchema};
use fleet_client::schema::column_for;
use std::collections::HashMap;

/// Draws whatever columns the daemon's schema declares, side by side, and puts each agent in the column the
/// schema maps its status to.
pub fn board(
    ui: &mut Ui,
    schema: &UiSchema,
    agents: &[Agent],
    now: i64,
    states: &mut HashMap<String, CardState>,
    selected: Option<&str>,
) -> Vec<Command> {
    let mut commands = Vec::new();
    if schema.columns.is_empty() {
        return commands;
    }
    let p = tokens(ui.visuals().dark_mode);
    let area = ui.available_rect_before_wrap();
    // Columns butt against each other; the dividers are painted lines, not gaps. The zero is only for the gap
    // between columns: each column gets the usual spacing back, or its cards' repos and badges would touch.
    let spacing = ui.spacing().item_spacing;
    ui.spacing_mut().item_spacing.x = 0.0;
    ui.columns(schema.columns.len(), |cols| {
        for (ui, def) in cols.iter_mut().zip(&schema.columns) {
            ui.spacing_mut().item_spacing = spacing;
            let here: Vec<&Agent> = agents
                .iter()
                .filter(|a| column_for(schema, a).is_some_and(|c| c.id == def.id))
                .collect();
            // `columns` lays each column out justified, and a wrapped label copies that into its text, stretching
            // the spaces of every row that breaks short of the width (task titles, empty hints).
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                commands.extend(column(ui, def, schema, &here, now, states, selected));
            });
        }
    });
    let n = schema.columns.len();
    for i in 1..n {
        let x = area.left() + area.width() * i as f32 / n as f32;
        ui.painter()
            .vline(x, area.y_range(), Stroke::new(1.0, p.border));
    }
    commands
}
