//! The board view: columns of cards, filtered by repo.
use super::{title, View};
use crate::components::{board, repo_filter, CardState, Command};
use crate::theme::tokens;
use eframe::egui::{self, Frame, Ui};
use fleet_api::{Agent, UiSchema};
use std::collections::HashMap;

/// Title and the repo filter over `(repo, agent count)`.
pub(crate) fn header(ui: &mut Ui, repos: &[(String, usize)], filter: &mut Vec<String>) {
    title(ui, View::Kanban);
    ui.add_space(6.0);
    repo_filter(ui, repos, filter);
}

/// The board in the space left after the panels. Draw the detail panel first: a side panel takes its width from
/// what the central panel has not yet claimed.
pub(crate) fn main(
    ui: &mut Ui,
    schema: &UiSchema,
    shown: &[Agent],
    now: i64,
    cards: &mut HashMap<String, CardState>,
    selected: Option<&str>,
) -> Vec<Command> {
    let bg = tokens(ui.visuals().dark_mode).bg;
    // No margin: the column headers sit flush under the header and the dividers reach the footer.
    egui::CentralPanel::default()
        .frame(Frame::new().fill(bg))
        .show(ui, |ui| board(ui, schema, shown, now, cards, selected))
        .inner
}
