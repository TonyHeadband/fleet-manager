use crate::theme::{mono, size, tokens};
use crate::widgets::{button, ButtonVariant};
use eframe::egui::{self, RichText, Ui};
use fleet_api::ToolError;

/// What the user asked for on a tool problem; the caller runs it (this component only draws).
#[derive(Debug, Clone, PartialEq)]
pub enum ToolAction {
    Install(String),
    CopyCommand(String),
    OpenLink(String),
}

/// One tool problem: the message, then the way out. `can_install` is whether the app may run the command itself;
/// without it, or without a command, the user gets the command to copy (if any) and the link.
pub fn tool_error_ui(ui: &mut Ui, e: &ToolError, can_install: bool) -> Option<ToolAction> {
    let p = tokens(ui.visuals().dark_mode);
    let mut action = None;
    ui.add(egui::Label::new(RichText::new(e.to_string()).color(p.text)).wrap());
    let Some(fix) = e.fix() else {
        ui.add(
            egui::Label::new(
                RichText::new("Log in again from the persona's setup.")
                    .size(size::SMALL)
                    .color(p.text_low),
            )
            .wrap(),
        );
        return None;
    };
    ui.add_space(4.0);
    if let Some(command) = &fix.install {
        if can_install && button(ui, "Install", ButtonVariant::Primary).clicked() {
            action = Some(ToolAction::Install(command.clone()));
        }
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if button(ui, "Copy", ButtonVariant::Ghost).clicked() {
                    action = Some(ToolAction::CopyCommand(command.clone()));
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(command).font(mono(size::CODE)).color(p.text),
                        )
                        .selectable(true)
                        .truncate(),
                    );
                });
            });
        });
    }
    if ui.link(&fix.link).clicked() {
        action = Some(ToolAction::OpenLink(fix.link.clone()));
    }
    action
}
