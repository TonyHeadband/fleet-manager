use crate::theme::{size, tokens};
use crate::widgets::chip;
use eframe::egui::{RichText, Ui};
use fleet_api::RepoInfo;

/// The product's repos picked so far, in the order they were picked. A product's repo set changes only in
/// Products, so nothing here adds a repo.
#[derive(Default)]
pub struct PickerState {
    pub chosen: Vec<String>,
}

/// The product's repos as toggle chips. Returns true when `chosen` changed. `id_salt` keeps two pickers on one
/// screen apart.
pub fn repo_picker(
    ui: &mut Ui,
    id_salt: &str,
    remembered: &[RepoInfo],
    st: &mut PickerState,
) -> bool {
    let p = tokens(ui.visuals().dark_mode);
    let mut changed = false;
    ui.push_id(id_salt, |ui| {
        if remembered.is_empty() {
            ui.label(
                RichText::new("This product has no repos; add them in Products")
                    .size(size::SMALL)
                    .color(p.text_low),
            );
            return;
        }
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
        });
    });
    changed
}
