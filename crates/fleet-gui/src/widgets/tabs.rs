use super::button::toggle;
use eframe::egui::Ui;

/// A row of toggle tabs; returns the id of a tab that was clicked and was not already selected.
pub fn tabs<Id: Copy + PartialEq>(ui: &mut Ui, tabs: &[(Id, &str)], selected: Id) -> Option<Id> {
    let mut picked = None;
    ui.horizontal(|ui| {
        for (id, label) in tabs {
            if toggle(ui, label, *id == selected).clicked() && *id != selected {
                picked = Some(*id);
            }
        }
    });
    picked
}
