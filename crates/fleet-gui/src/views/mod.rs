//! The views the drawer's nav switches between. A view owns its content header, its main area and whether the
//! selected agent's detail panel shows; the drawer, the content footer and the selection belong to the app.
pub mod kanban;
pub mod settings;

use crate::theme::{sans, size, tokens, Weight};
use eframe::egui::{RichText, Ui};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Kanban,
    Settings,
}

impl View {
    /// The nav entry and the header title.
    pub fn label(self) -> &'static str {
        match self {
            View::Kanban => "Kanban",
            View::Settings => "Settings",
        }
    }
}

/// The view's name at the start of the content header.
pub(crate) fn title(ui: &mut Ui, view: View) {
    let p = tokens(ui.visuals().dark_mode);
    ui.label(
        RichText::new(view.label())
            .font(sans(ui, size::TITLE, Weight::Medium))
            .color(p.text_high),
    );
}
