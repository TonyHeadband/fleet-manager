//! Settings → General: choices that apply at once and are saved at once, and the projects and personas folders,
//! which need Save because it restarts the daemon.
use super::{SettingsAction, SettingsState};
use crate::components::{browse, FolderPicker};
use crate::theme::{sans, size, tokens, Weight};
use crate::widgets::{button, chip, path_edit, ButtonVariant};
use eframe::egui::{self, accesskit::Role, Response, RichText, Ui, WidgetInfo};
use fleet_client::config::{CloseChoice, ThemeChoice};

/// A choice the user just made; the app applies and saves it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GeneralChange {
    Theme(ThemeChoice),
    /// `None` is Ask.
    CloseChoice(Option<CloseChoice>),
    StickyAlerts(bool),
}

/// A folder field (Projects or Personas): the app's text and Browse… dialog, why Save cannot run now, if it
/// cannot, and whether a restart this field is concerned by is running.
pub struct FolderField<'a> {
    pub text: &'a mut String,
    pub picker: &'a mut FolderPicker,
    pub blocked: Option<&'static str>,
    pub restarting: bool,
}

/// What tells one folder row from the other.
struct Row {
    heading: &'static str,
    id: &'static str,
    /// The Save button's name for assistive tech and tests when "Save" alone is ambiguous on the tab.
    save_name: Option<&'static str>,
    note: &'static str,
}

const TASKS: Row = Row {
    heading: "Projects folder",
    id: "settings_projects_folder",
    save_name: None,
    note: "Applies to new projects only. Saving restarts the daemon; running agents keep going.",
};

// The daemon reads `personas_dir` only at startup and never moves a persona's folder: its logins live there.
const PERSONAS: Row = Row {
    heading: "Personas folder",
    id: "settings_personas_folder",
    save_name: Some("Save personas folder"),
    note: "Saving restarts the daemon. Existing personas' folders, with their Claude and GitHub logins, are not moved: move them by hand or log the personas in again.",
};

const THEMES: [ThemeChoice; 3] = [ThemeChoice::System, ThemeChoice::Dark, ThemeChoice::Light];
const CLOSE: [(Option<CloseChoice>, &str); 3] = [
    (None, "Ask"),
    (Some(CloseChoice::KeepRunning), "Keep daemon running"),
    (Some(CloseChoice::StopDaemon), "Stop daemon"),
];

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
            if chip(ui, label, choice == state.close_choice).clicked()
                && choice != state.close_choice
            {
                change = Some(GeneralChange::CloseChoice(choice));
            }
        }
    });
    ui.add_space(16.0);
    heading(ui, "Footer");
    let mut sticky = state.sticky_alerts;
    if ui
        .checkbox(
            &mut sticky,
            "Keep errors and warnings in the footer until dismissed",
        )
        .clicked()
    {
        change = Some(GeneralChange::StickyAlerts(sticky));
    }
    let mut action = change.map(SettingsAction::Change);
    if let Some(tasks) = state.tasks {
        ui.add_space(16.0);
        if folder(ui, &TASKS, tasks) {
            action = Some(SettingsAction::SaveTasksRoot);
        }
    }
    if let Some(personas) = state.personas {
        ui.add_space(16.0);
        if folder(ui, &PERSONAS, personas) {
            action = Some(SettingsAction::SavePersonasDir);
        }
    }
    action
}

/// The field, Browse… and Save, and the note. Trust is per persona, in its Claude login. True when Save was clicked.
fn folder(ui: &mut Ui, row: &Row, f: FolderField<'_>) -> bool {
    let p = tokens(ui.visuals().dark_mode);
    let label = heading(ui, row.heading);
    let mut save = false;
    // Wide enough for a real path, but not the whole window on a wide screen.
    ui.allocate_ui(egui::vec2(ui.available_width().min(640.0), 0.0), |ui| {
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // From the right, so the field takes exactly the rest of the row.
                let enabled = f.blocked.is_none();
                let clicked = ui
                    .add_enabled_ui(enabled, |ui| button(ui, "Save", ButtonVariant::Primary))
                    .inner;
                if let Some(name) = row.save_name {
                    clicked.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, name));
                }
                save = clicked
                    .on_disabled_hover_text(f.blocked.unwrap_or_default())
                    .clicked();
                let picked = browse(ui, f.picker, f.text);
                let (id, width) = (ui.make_persistent_id(row.id), ui.available_width());
                path_edit(ui, id, f.text, width, None, "", picked).labelled_by(label.id);
            });
        });
        ui.add_space(4.0);
        let note = if f.restarting {
            "Restarting the daemon…"
        } else {
            row.note
        };
        ui.label(RichText::new(note).size(size::SMALL).color(p.text_low));
    });
    save
}

pub(super) fn heading(ui: &mut Ui, text: &str) -> Response {
    let p = tokens(ui.visuals().dark_mode);
    let r = ui.label(
        RichText::new(text)
            .font(sans(ui, size::TITLE, Weight::Medium))
            .color(p.text_high),
    );
    ui.add_space(6.0);
    r
}
