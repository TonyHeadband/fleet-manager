//! Settings: the app's and the daemon's options, one tab per area.
mod general;

pub use general::GeneralChange;

use super::{title, View};
use crate::theme::{size, tokens};
use crate::widgets::tab_bar;
use eframe::egui::{self, Frame, Margin, RichText, Ui};
use fleet_client::config::{CloseChoice, ThemeChoice};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SettingsTab {
    General,
    Daemon,
    Log,
}

const TABS: [(SettingsTab, &str); 3] = [(SettingsTab::General, "General"), (SettingsTab::Daemon, "Daemon"), (SettingsTab::Log, "Log")];

pub(crate) fn header(ui: &mut Ui) {
    title(ui, View::Settings);
}

/// The tab bar and the selected tab. `tab` is the app's, so the last tab is kept while the app runs.
pub(crate) fn main(ui: &mut Ui, tab: &mut SettingsTab, theme: ThemeChoice, close_choice: Option<CloseChoice>, sticky_alerts: bool) -> Option<GeneralChange> {
    let p = tokens(ui.visuals().dark_mode);
    let mut change = None;
    egui::CentralPanel::default().frame(Frame::new().fill(p.bg)).show(ui, |ui| {
        if let Some(picked) = tab_bar(ui, &TABS, *tab) {
            *tab = picked;
        }
        Frame::new().inner_margin(Margin::symmetric(22, 18)).show(ui, |ui| match *tab {
            SettingsTab::General => change = general::general(ui, theme, close_choice, sticky_alerts),
            SettingsTab::Daemon | SettingsTab::Log => {
                ui.label(RichText::new("Nothing here yet.").size(size::SMALL).color(p.text_low));
            }
        });
    });
    change
}
