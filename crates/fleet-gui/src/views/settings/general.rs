//! Settings → General: choices that apply at once and are saved at once.
use crate::theme::{sans, size, tokens, Weight};
use crate::widgets::chip;
use eframe::egui::{RichText, Ui};
use fleet_client::config::{CloseChoice, ThemeChoice};

/// A choice the user just made; the app applies and saves it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GeneralChange {
    Theme(ThemeChoice),
    /// `None` is Ask.
    CloseChoice(Option<CloseChoice>),
}

const THEMES: [ThemeChoice; 3] = [ThemeChoice::System, ThemeChoice::Dark, ThemeChoice::Light];
const CLOSE: [(Option<CloseChoice>, &str); 3] = [(None, "Ask"), (Some(CloseChoice::KeepRunning), "Keep daemon running"), (Some(CloseChoice::StopDaemon), "Stop daemon")];

pub(super) fn general(ui: &mut Ui, theme: ThemeChoice, close_choice: Option<CloseChoice>) -> Option<GeneralChange> {
    let mut change = None;
    heading(ui, "Theme");
    ui.horizontal(|ui| {
        for choice in THEMES {
            if chip(ui, choice.label(), choice == theme).clicked() && choice != theme {
                change = Some(GeneralChange::Theme(choice));
            }
        }
    });
    ui.add_space(16.0);
    heading(ui, "Closing the window");
    ui.horizontal(|ui| {
        for (choice, label) in CLOSE {
            if chip(ui, label, choice == close_choice).clicked() && choice != close_choice {
                change = Some(GeneralChange::CloseChoice(choice));
            }
        }
    });
    change
}

fn heading(ui: &mut Ui, text: &str) {
    let p = tokens(ui.visuals().dark_mode);
    ui.label(RichText::new(text).font(sans(ui, size::TITLE, Weight::Medium)).color(p.text_high));
    ui.add_space(6.0);
}
