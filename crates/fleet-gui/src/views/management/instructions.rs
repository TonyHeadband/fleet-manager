//! Management → Instructions: the text every agent's CLAUDE.md starts with, whatever its persona. Its input is the
//! daemon's `instructions` form; the pane keeps its own Save and Revert.
use super::personas::error_line;
use super::{config_pane, save_bar, ConfigState, ManagementAction};
use crate::components::form;
use eframe::egui::Ui;
use fleet_api::FormDef;

const WIDTH: f32 = 760.0;

pub(super) fn instructions(
    ui: &mut Ui,
    cfg: &mut ConfigState,
    def: &FormDef,
) -> Option<ManagementAction> {
    config_pane(ui, cfg, WIDTH, |ui, cfg| {
        if let Some(input) = def.inputs().find(|i| i.key == "instructions") {
            form::input(ui, input, &mut cfg.instructions, None);
        }
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
