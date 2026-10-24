use super::card::CardState;
use super::column::column;
use super::Command;
use eframe::egui::Ui;
use fleet_api::{Agent, UiSchema};
use fleet_client::schema::column_for;
use std::collections::HashMap;

/// Draws whatever columns the daemon's schema declares, side by side, and puts each agent in the column the
/// schema maps its status to.
pub fn board(ui: &mut Ui, schema: &UiSchema, agents: &[Agent], now: i64, states: &mut HashMap<String, CardState>) -> Vec<Command> {
    let mut commands = Vec::new();
    if schema.columns.is_empty() {
        return commands;
    }
    ui.columns(schema.columns.len(), |cols| {
        for (ui, def) in cols.iter_mut().zip(&schema.columns) {
            let here: Vec<&Agent> = agents.iter().filter(|a| column_for(schema, a).is_some_and(|c| c.id == def.id)).collect();
            commands.extend(column(ui, def, schema, &here, now, states));
        }
    });
    commands
}
