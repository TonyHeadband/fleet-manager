//! Settings → General: choices that apply at once and are saved at once, and the projects folder, which needs
//! Save because it restarts the daemon.
use super::{SettingsAction, SettingsState};
use crate::components::{browse, trust_hint, FolderPicker};
use crate::theme::{sans, size, tokens, Weight};
use crate::widgets::{button, chip, path_edit, ButtonVariant};
use eframe::egui::{self, Response, RichText, Ui};
use fleet_client::config::{CloseChoice, ThemeChoice};

/// A choice the user just made; the app applies and saves it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GeneralChange {
    Theme(ThemeChoice),
    /// `None` is Ask.
    CloseChoice(Option<CloseChoice>),
    StickyAlerts(bool),
}

/// The Projects folder field: the app's text and Browse… dialog, and why Save cannot run now, if it cannot.
pub struct ProjectsFolder<'a> {
    pub text: &'a mut String,
    pub picker: &'a mut FolderPicker,
    pub blocked: Option<&'static str>,
    pub restarting: bool,
}

const THEMES: [ThemeChoice; 3] = [ThemeChoice::System, ThemeChoice::Dark, ThemeChoice::Light];
const CLOSE: [(Option<CloseChoice>, &str); 3] = [(None, "Ask"), (Some(CloseChoice::KeepRunning), "Keep daemon running"), (Some(CloseChoice::StopDaemon), "Stop daemon")];

pub(super) fn general(ui: &mut Ui, state: SettingsState<'_>) -> Option<SettingsAction> {
    let mut change = None;
    heading(ui, "Theme");
    ui.horizontal(|ui| {
        for choice in THEMES {
            if chip(ui, choice.label(), choice == state.theme).clicked() && choice != state.theme {
                change = Some(GeneralChange::Theme(choice));
            }
        }
    });
    ui.add_space(16.0);
    heading(ui, "Closing the window");
    ui.horizontal(|ui| {
        for (choice, label) in CLOSE {
            if chip(ui, label, choice == state.close_choice).clicked() && choice != state.close_choice {
                change = Some(GeneralChange::CloseChoice(choice));
            }
        }
    });
    ui.add_space(16.0);
    heading(ui, "Footer");
    let mut sticky = state.sticky_alerts;
    if ui.checkbox(&mut sticky, "Keep errors and warnings in the footer until dismissed").clicked() {
        change = Some(GeneralChange::StickyAlerts(sticky));
    }
    let mut action = change.map(SettingsAction::Change);
    if let Some(projects) = state.projects {
        ui.add_space(16.0);
        if projects_folder(ui, projects) {
            action = Some(SettingsAction::SaveProjectsRoot);
        }
    }
    action
}

/// The field, Browse… and Save, the note and the trust line. True when Save was clicked.
fn projects_folder(ui: &mut Ui, f: ProjectsFolder<'_>) -> bool {
    let p = tokens(ui.visuals().dark_mode);
    let label = heading(ui, "Projects folder");
    let mut save = false;
    // Wide enough for a real path, but not the whole window on a wide screen.
    ui.allocate_ui(egui::vec2(ui.available_width().min(640.0), 0.0), |ui| {
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // From the right, so the field takes exactly the rest of the row.
                let enabled = f.blocked.is_none();
                let clicked = ui.add_enabled_ui(enabled, |ui| button(ui, "Save", ButtonVariant::Primary)).inner;
                save = clicked.on_disabled_hover_text(f.blocked.unwrap_or_default()).clicked();
                let picked = browse(ui, f.picker, f.text);
                let (id, width) = (ui.make_persistent_id("settings_projects_folder"), ui.available_width());
                path_edit(ui, id, f.text, width, None, "", picked).labelled_by(label.id);
            });
        });
        ui.add_space(4.0);
        let note = if f.restarting { "Restarting the daemon…" } else { "Applies to new projects only. Saving restarts the daemon; running agents keep going." };
        ui.label(RichText::new(note).size(size::SMALL).color(p.text_low));
        ui.add_space(8.0);
        trust_hint(ui, f.text);
    });
    save
}

pub(super) fn heading(ui: &mut Ui, text: &str) -> Response {
    let p = tokens(ui.visuals().dark_mode);
    let r = ui.label(RichText::new(text).font(sans(ui, size::TITLE, Weight::Medium)).color(p.text_high));
    ui.add_space(6.0);
    r
}
