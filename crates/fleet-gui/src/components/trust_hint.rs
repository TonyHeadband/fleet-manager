use crate::shell::trust_command;
use crate::theme::{mono, size, tokens};
use crate::widgets::{button, ButtonVariant};
use eframe::egui::{self, RichText, Ui};

/// How to trust `root` for Claude once: the hint, the shell command and its Copy button.
pub fn trust_hint(ui: &mut Ui, root: &str) {
    let p = tokens(ui.visuals().dark_mode);
    let command = trust_command(root);
    // Claude's trust dialog pre-selects "No, exit": "accept" or typing 1 there ends the session.
    ui.add(
        egui::Label::new(RichText::new("Trust it once for Claude: run this, choose \"Yes, I trust this folder\" (the second option), then /exit.").size(size::SMALL).color(p.text_low))
            .wrap(),
    );
    ui.add_space(4.0);
    // Copy first, from the right, so it keeps its size; a long command is cut instead, with egui's elision tooltip
    // showing it whole.
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if button(ui, "Copy", ButtonVariant::Ghost).clicked() {
                ui.ctx().copy_text(command.clone());
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(
                    egui::Label::new(RichText::new(&command).font(mono(size::CODE)).color(p.text))
                        .truncate(),
                );
            });
        });
    });
}
