use super::{modal_frame, repo_picker, FolderPicker, PickerState};
use crate::theme::{mono, sans, size, tokens, Weight};
use crate::widgets::{button, icon_button, ButtonVariant, Icon};
use eframe::egui::{self, Align, Key, Layout, Modifiers, RichText, TextEdit, TextStyle, Ui};
use fleet_api::RepoInfo;

const WIDTH: f32 = 440.0;
const PROMPT_ROWS: usize = 4;

pub struct SpawnState {
    /// The dialog is showing; the drawer's "New project" button sets it.
    pub open: bool,
    pub picker: PickerState,
    pub prompt: String,
    /// A spawn is in flight; the app sets this when it starts the call and clears it when the call returns.
    pub busy: bool,
    pub error: Option<String>,
    /// The goal the last spawn sent. A success clears the dialog only while it still holds this text, so a
    /// goal the user reopened and changed in the meantime is kept.
    pub sent: Option<String>,
}

impl SpawnState {
    /// `browse` is the dialog's repo picker, built by the app with the paths it knows.
    pub fn new(browse: FolderPicker) -> SpawnState {
        SpawnState {
            open: false,
            picker: PickerState::new(browse),
            prompt: String::new(),
            busy: false,
            error: None,
            sent: None,
        }
    }
}

fn caption(ui: &mut Ui, text: &str) -> egui::Response {
    let low = tokens(ui.visuals().dark_mode).text_low;
    ui.label(
        RichText::new(text)
            .font(sans(ui, size::SMALL, Weight::Medium))
            .color(low),
    )
}

/// The New project dialog: the goal, an optional repo picker and Start. Draws nothing unless `st.open`. Returns
/// `(repos, goal)` when the user starts the project; the app clears the goal when the spawn succeeds, so a
/// failure keeps what was typed. Closing by any route keeps the goal for next time.
pub fn new_agent_dialog(
    ctx: &egui::Context,
    repos: &[RepoInfo],
    st: &mut SpawnState,
) -> Option<(Vec<String>, String)> {
    let opened = egui::Id::unique("new_agent_opened");
    if !st.open {
        ctx.data_mut(|d| d.remove_temp::<bool>(opened));
        return None;
    }
    // The prompt takes the focus on the first frame of each opening only, so later frames do not steal it back.
    let first_frame = ctx.data_mut(|d| d.get_temp::<bool>(opened)).is_none();
    ctx.data_mut(|d| d.insert_temp(opened, true));

    let p = tokens(ctx.global_style().visuals.dark_mode);
    let frame = modal_frame(ctx);
    // `WIDTH` is the dialog's outside; the content gets what the frame's margin and stroke leave.
    let inner_width = WIDTH - frame.total_margin().sum().x;
    let (mut close, mut started) = (false, None);
    let modal = egui::Modal::new(egui::Id::unique("new_agent_dialog"))
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_width(inner_width);
            // Taken before the TextEdit is drawn, or the edit would insert a newline for it.
            let ctrl_enter = ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter));

            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("New project")
                        .text_style(TextStyle::Heading)
                        .color(p.text_high),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    close |= icon_button(ui, Icon::Close, "Close").clicked();
                });
            });
            ui.add_space(12.0);

            let label = caption(ui, "Goal");
            ui.add_space(4.0);
            // Names the box for screen readers and for the UI tests, which refuse unnamed inputs.
            let edit = ui
                .add(
                    TextEdit::multiline(&mut st.prompt)
                        .desired_rows(PROMPT_ROWS)
                        .desired_width(f32::INFINITY),
                )
                .labelled_by(label.id);
            if first_frame {
                edit.request_focus();
            }
            ui.add_space(10.0);

            caption(ui, "Repos (optional)");
            ui.add_space(4.0);
            repo_picker(ui, "new_project_repos", repos, &mut st.picker);
            ui.add_space(12.0);

            if let Some(e) = &st.error {
                ui.label(RichText::new(e).size(size::SMALL).color(p.danger));
                ui.add_space(8.0);
            }

            let ready = !st.busy && !st.prompt.trim().is_empty();
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Ctrl+Enter")
                        .font(mono(size::META))
                        .color(p.text_low),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let start = ui
                        .add_enabled_ui(ready, |ui| {
                            button(ui, "Start project", ButtonVariant::Primary)
                        })
                        .inner;
                    if (start.clicked() || ctrl_enter) && ready {
                        st.picker.commit_typed();
                        started = Some((st.picker.chosen.clone(), st.prompt.clone()));
                    }
                    close |= button(ui, "Cancel", ButtonVariant::Default).clicked();
                });
            });
        });
    // Escape, a click outside and the dialog's own close request all land here; none of them clears the prompt.
    if started.is_none() && (close || modal.should_close()) {
        st.open = false;
    }
    started
}
