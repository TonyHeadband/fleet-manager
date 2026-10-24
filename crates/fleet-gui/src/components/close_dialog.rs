use super::modal_frame;
use crate::theme::{size, tokens};
use crate::widgets::{button, ButtonVariant};
use eframe::egui::{self, RichText, TextStyle};
use fleet_client::close::{dialog_text, Summary};
use fleet_client::config::CloseChoice;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogResult {
    Choose(CloseChoice),
    /// Leave the window open and change nothing. Escape and clicking outside mean this, never "stop the daemon".
    Cancel,
}

/// Asks what closing the window should do about the daemon, stating the consequences with the live numbers.
/// `remember` is the state of the "remember my choice" box. Returns the user's answer on the frame it is given.
pub fn close_dialog(ctx: &egui::Context, summary: &Summary, remember: &mut bool) -> Option<DialogResult> {
    let mut result = None;
    let p = tokens(ctx.global_style().visuals.dark_mode);
    let modal = egui::Modal::new(egui::Id::unique("close_dialog")).frame(modal_frame(ctx)).show(ctx, |ui| {
        ui.set_max_width(520.0);
        ui.label(RichText::new("Closing fleet-manager").text_style(TextStyle::Heading).color(p.text_high));
        ui.add_space(6.0);
        ui.label(RichText::new(dialog_text(summary)).color(p.text));
        ui.add_space(8.0);
        ui.checkbox(remember, "Remember my choice");
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            let keep = button(ui, "Keep the daemon running", ButtonVariant::Primary);
            // The safe choice is the default: it has the focus unless the user already moved it.
            if ctx.memory(|m| m.focused().is_none()) {
                keep.request_focus();
            }
            if keep.clicked() {
                result = Some(DialogResult::Choose(CloseChoice::KeepRunning));
            }
            if button(ui, "Stop the daemon too", ButtonVariant::QuietDanger).clicked() {
                result = Some(DialogResult::Choose(CloseChoice::StopDaemon));
            }
            if button(ui, "Cancel", ButtonVariant::Default).clicked() {
                result = Some(DialogResult::Cancel);
            }
        });
        ui.add_space(8.0);
        ui.label(RichText::new("You can change this later from the top bar.").size(size::SMALL).color(p.text_low));
    });
    if result.is_none() && modal.should_close() {
        result = Some(DialogResult::Cancel);
    }
    result
}
