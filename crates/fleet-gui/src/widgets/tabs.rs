use crate::theme::{sans, size, tokens, Weight};
use eframe::egui::{self, vec2, Rect, Sense, Ui, WidgetInfo};

const UNDERLINE: f32 = 2.0;

/// A row of underline tabs; returns the id of a tab that was clicked and was not already selected.
pub fn tabs<Id: Copy + PartialEq>(ui: &mut Ui, tabs: &[(Id, &str)], selected: Id) -> Option<Id> {
    let p = tokens(ui.visuals().dark_mode);
    let font = sans(ui, size::BUTTON, Weight::Regular);
    let mut picked = None;
    ui.horizontal(|ui| {
        for (id, label) in tabs {
            let is_selected = *id == selected;
            let galley = ui.painter().layout_no_wrap((*label).to_owned(), font.clone(), p.text_low);
            let (rect, response) = ui.allocate_exact_size(galley.size() + vec2(16.0, 10.0), Sense::click());
            let colour = if is_selected || response.hovered() { p.text_high } else { p.text_low };
            ui.painter().galley(rect.center() - galley.size() / 2.0, galley, colour);
            if is_selected {
                let bar = Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - UNDERLINE), rect.right_bottom());
                ui.painter().rect_filled(bar, 0.0, p.text_high);
            }
            // The underline is the only visual cue; assistive tech and tests read the state from here.
            response.widget_info(|| WidgetInfo::selected(egui::accesskit::Role::Button, ui.is_enabled(), is_selected, label));
            if response.clicked() && !is_selected {
                picked = Some(*id);
            }
        }
    });
    picked
}
