use crate::widgets::toggle;
use eframe::egui::Ui;

/// One toggle per repo. An empty selection means no filter. Returns true when this frame changed it;
/// repos are kept in the order they were clicked.
pub fn repo_filter(ui: &mut Ui, repos: &[String], selected: &mut Vec<String>) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        for repo in repos {
            let on = selected.contains(repo);
            if toggle(ui, repo, on).clicked() {
                if on {
                    selected.retain(|s| s != repo);
                } else {
                    selected.push(repo.clone());
                }
                changed = true;
            }
        }
    });
    changed
}
