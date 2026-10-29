//! Management → Agent settings: the permissions and environment every agent gets, whatever its persona.
use super::personas::{error_line, note};
use super::{config_pane, save_bar, ConfigState, ManagementAction};
use crate::components::caption;
use crate::theme::{mono, size};
use crate::widgets::{button, small_icon_button, ButtonVariant, Icon};
use eframe::egui::{TextEdit, Ui, WidgetInfo};

const WIDTH: f32 = 620.0;

/// A single-line field named `name` for assistive tech and tests (rows share a caption, so `labelled_by` cannot
/// tell them apart).
fn row_edit(ui: &mut Ui, text: &mut String, name: &str, hint: &str, width: f32) {
    let r = ui.add(
        TextEdit::singleline(text)
            .font(mono(size::CODE))
            .hint_text(hint)
            .desired_width(width),
    );
    let (value, hint) = (text.clone(), hint.to_string());
    r.widget_info(|| {
        let mut info = WidgetInfo::text_edit(true, "", value.clone(), hint.clone());
        info.label = Some(name.to_string());
        info
    });
}

/// One list of rules: a text row each with a remove button, then Add.
fn rules(ui: &mut Ui, title: &str, noun: &str, hint: &str, rows: &mut Vec<String>) {
    caption(ui, title);
    ui.add_space(4.0);
    let mut remove = None;
    for (i, row) in rows.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let w = ui.available_width() - 28.0;
            row_edit(ui, row, &format!("{noun} {}", i + 1), hint, w);
            if small_icon_button(ui, Icon::Close, &format!("Remove {noun} {}", i + 1), false)
                .clicked()
            {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        rows.remove(i);
    }
    if button(ui, &format!("Add {noun}"), ButtonVariant::Default).clicked() {
        rows.push(String::new());
    }
    ui.add_space(16.0);
}

fn env(ui: &mut Ui, rows: &mut Vec<(String, String)>) {
    caption(ui, "Environment");
    ui.add_space(2.0);
    note(
        ui,
        "A repo's `env:` in `.fleet/workflow.yml` wins over these; Products shows what each repo's tasks run with.",
    );
    ui.add_space(4.0);
    let mut remove = None;
    for (i, (key, value)) in rows.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 36.0) / 2.0;
            row_edit(ui, key, &format!("Variable name {}", i + 1), "NAME", w);
            row_edit(ui, value, &format!("Variable value {}", i + 1), "value", w);
            if small_icon_button(
                ui,
                Icon::Close,
                &format!("Remove variable {}", i + 1),
                false,
            )
            .clicked()
            {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        rows.remove(i);
    }
    if button(ui, "Add variable", ButtonVariant::Default).clicked() {
        rows.push((String::new(), String::new()));
    }
    ui.add_space(16.0);
}

pub(super) fn agent_settings(ui: &mut Ui, cfg: &mut ConfigState) -> Option<ManagementAction> {
    config_pane(ui, cfg, WIDTH, |ui, cfg| {
        if let Some(e) = &cfg.saved.settings_error {
            error_line(
                ui,
                &format!("{e}. Saving valid settings replaces the file."),
            );
            ui.add_space(8.0);
        }
        note(
            ui,
            "Written into each agent's Claude settings, on top of what its persona and task add.",
        );
        ui.add_space(12.0);
        rules(
            ui,
            "Allow rules",
            "allow rule",
            "Bash(git status:*)",
            &mut cfg.settings.allow,
        );
        rules(
            ui,
            "Deny rules",
            "deny rule",
            "Read(./.env)",
            &mut cfg.settings.deny,
        );
        env(ui, &mut cfg.settings.env);
        if let Some(e) = &cfg.settings_error {
            error_line(ui, e);
        }
        ui.add_space(10.0);
        let (save, revert) = save_bar(
            ui,
            cfg.settings_dirty(),
            cfg.saved.settings_error.is_some(),
            cfg.saving.is_some(),
        );
        if save {
            Some(ManagementAction::SaveSettings)
        } else if revert {
            Some(ManagementAction::RevertSettings)
        } else {
            None
        }
    })
}
