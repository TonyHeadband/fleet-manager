use crate::theme::palette;
use crate::widgets::{button, ButtonVariant};
use eframe::egui::{self, RichText, TextEdit, Ui};

#[derive(Default)]
pub struct SpawnState {
    pub repo: String,
    pub prompt: String,
    /// A spawn is in flight; the app sets this when it starts the call and clears it when the call returns.
    pub busy: bool,
    pub error: Option<String>,
}

/// The repo picker, the prompt and the Start button. Returns `(repo, prompt)` when Start is clicked; the app
/// clears the prompt when the spawn succeeds, so a failure keeps what was typed.
pub fn spawn_form(ui: &mut Ui, repos: &[String], st: &mut SpawnState) -> Option<(String, String)> {
    if !repos.contains(&st.repo) {
        st.repo = repos.first().cloned().unwrap_or_default();
    }
    let mut started = None;
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.horizontal(|ui| {
            egui::ComboBox::from_label("Repo").selected_text(st.repo.clone()).show_ui(ui, |ui| {
                for r in repos {
                    ui.selectable_value(&mut st.repo, r.clone(), r);
                }
            });
            ui.vertical(|ui| {
                let label = ui.label("Prompt");
                // Names the box for screen readers and for the UI tests, which refuse unnamed inputs.
                ui.add(TextEdit::multiline(&mut st.prompt).desired_rows(2).desired_width(f32::INFINITY)).labelled_by(label.id);
            });
        });
        let ready = !st.busy && !st.repo.is_empty() && !st.prompt.trim().is_empty();
        if ui.add_enabled_ui(ready, |ui| button(ui, "Start agent", ButtonVariant::Default)).inner.clicked() && ready {
            started = Some((st.repo.clone(), st.prompt.clone()));
        }
        if let Some(e) = &st.error {
            ui.label(RichText::new(e).color(palette(ui.visuals().dark_mode).danger));
        }
    });
    started
}
