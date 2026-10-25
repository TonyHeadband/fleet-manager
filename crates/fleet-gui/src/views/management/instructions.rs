//! Management → Instructions: the text every agent's CLAUDE.md starts with, whatever its persona.
use super::personas::{error_line, note};
use super::{config_pane, save_bar, ConfigState, ManagementAction};
use crate::components::caption;
use crate::theme::{mono, size};
use eframe::egui::{TextEdit, Ui};

const WIDTH: f32 = 760.0;
const ROWS: usize = 24;

pub(super) fn instructions(ui: &mut Ui, cfg: &mut ConfigState) -> Option<ManagementAction> {
    config_pane(ui, cfg, WIDTH, |ui, cfg| {
        let label = caption(ui, "Instructions for every agent");
        ui.add_space(4.0);
        note(
            ui,
            "Markdown, put at the top of each agent's CLAUDE.md under \"# Instructions for every agent\".",
        );
        ui.add_space(6.0);
        ui.add(
            TextEdit::multiline(&mut cfg.instructions)
                .font(mono(size::CODE))
                .desired_rows(ROWS)
                .desired_width(f32::INFINITY),
        )
        .labelled_by(label.id);
        if let Some(e) = &cfg.instructions_error {
            error_line(ui, e);
        }
        ui.add_space(10.0);
        let (save, revert) = save_bar(ui, cfg.instructions_dirty(), false, cfg.saving.is_some());
        if save {
            Some(ManagementAction::SaveInstructions)
        } else if revert {
            Some(ManagementAction::RevertInstructions)
        } else {
            None
        }
    })
}
