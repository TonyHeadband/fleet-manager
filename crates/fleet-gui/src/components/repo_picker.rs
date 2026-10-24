use crate::theme::{sans, size, tokens, Weight};
use crate::widgets::{badge, button, chip, ButtonVariant, icon_button, BadgeTone, Icon};
use eframe::egui::{Key, RichText, TextEdit, Ui};
use fleet_api::RepoInfo;

#[derive(Default)]
pub struct PickerState {
    /// Remembered names and typed entries, in the order they were added.
    pub chosen: Vec<String>,
    /// The "path or git URL" field's text.
    pub typed: String,
}

impl PickerState {
    /// Folds the field's trimmed text into `chosen` unless it is blank or already there, then clears the field.
    /// Add, Enter and Start share it so text left in the field is never silently dropped. True when `chosen` grew.
    pub fn commit_typed(&mut self) -> bool {
        let t = self.typed.trim().to_string();
        self.typed.clear();
        if t.is_empty() || self.chosen.contains(&t) {
            return false;
        }
        self.chosen.push(t);
        true
    }
}

/// Remembered repos as toggle chips plus a "path or git URL" field with Add. Returns true when `chosen` changed.
/// `id_salt` keeps two pickers on one screen apart.
pub fn repo_picker(ui: &mut Ui, id_salt: &str, remembered: &[RepoInfo], st: &mut PickerState) -> bool {
    let p = tokens(ui.visuals().dark_mode);
    let mut changed = false;
    ui.push_id(id_salt, |ui| {
        ui.horizontal_wrapped(|ui| {
            for r in remembered {
                let on = st.chosen.contains(&r.name);
                if chip(ui, &r.name, on).clicked() {
                    if on {
                        st.chosen.retain(|c| *c != r.name);
                    } else {
                        st.chosen.push(r.name.clone());
                    }
                    changed = true;
                }
            }
            // Typed entries only: a remembered name's chip is always drawn above, selected or not.
            let typed_entries: Vec<String> = st.chosen.iter().filter(|c| !remembered.iter().any(|r| &r.name == *c)).cloned().collect();
            for entry in typed_entries {
                ui.horizontal(|ui| {
                    badge(ui, &entry, "Typed repo", BadgeTone::Neutral);
                    if icon_button(ui, Icon::Close, &format!("Remove {entry}")).clicked() {
                        st.chosen.retain(|c| *c != entry);
                        changed = true;
                    }
                });
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let label = ui.label(RichText::new("Path or git URL").font(sans(ui, size::SMALL, Weight::Regular)).color(p.text_low));
            // Reserve room for Add so the field takes the rest of the row.
            let width = (ui.available_width() - 64.0).max(80.0);
            let edit = ui.add(TextEdit::singleline(&mut st.typed).desired_width(width).hint_text("path or git URL")).labelled_by(label.id);
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            let add = button(ui, "Add", ButtonVariant::Default);
            if add.clicked() || enter {
                changed |= st.commit_typed();
                if enter {
                    edit.request_focus();
                }
            }
        });
    });
    changed
}
