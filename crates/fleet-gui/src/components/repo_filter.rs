use crate::theme::{mono, sans, size, tokens, Weight};
use eframe::egui::{self, accesskit::Role, Align, Layout, PopupCloseBehavior, RichText, Ui, WidgetInfo};

/// What the closed dropdown shows after "Repo".
pub fn filter_summary(selected: &[String]) -> String {
    match selected {
        [] => "All repos".to_string(),
        [one] => one.clone(),
        many => format!("{} repos", many.len()),
    }
}

const MENU_MIN_WIDTH: f32 = 200.0;

/// A multi-select dropdown over `(repo, agent count)`. An empty selection means no filter. Returns true when this
/// frame changed it; repos are kept in the order they were clicked.
pub fn repo_filter(ui: &mut Ui, repos: &[(String, usize)], selected: &mut Vec<String>) -> bool {
    let p = tokens(ui.visuals().dark_mode);
    let label = RichText::new("Repo").font(sans(ui, size::SMALL, Weight::Regular)).color(p.text_low);
    let summary = filter_summary(selected);
    let mut changed = false;
    let combo = egui::ComboBox::from_id_salt("repo_filter")
        .selected_text((label, summary))
        // The default closes the menu on every click, which would make picking several repos a chore.
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .show_ui(ui, |ui| {
            ui.set_min_width(MENU_MIN_WIDTH);
            let mut all = selected.is_empty();
            if ui.checkbox(&mut all, "All repos").clicked() {
                // Unticking "All repos" has nothing to fall back on, so the row only ever clears.
                changed = !selected.is_empty();
                selected.clear();
            }
            ui.separator();
            for (repo, count) in repos {
                let mut on = selected.contains(repo);
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut on, repo).clicked() {
                        if on {
                            selected.push(repo.clone());
                        } else {
                            selected.retain(|s| s != repo);
                        }
                        changed = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(count.to_string()).font(mono(size::META)).color(p.text_low));
                    });
                });
            }
        });
    // The "Repo" in the button is part of its value, not a name; a screen reader and the UI tests need one.
    combo.response.widget_info(|| WidgetInfo::labeled(Role::ComboBox, ui.is_enabled(), "Repo filter"));
    changed
}
