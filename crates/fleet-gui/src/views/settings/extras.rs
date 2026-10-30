//! Settings → Extras: the optional tools, each with its state and Install. With none yet, the required tools'
//! state, read-only, and the way to the wizard that installs them.
use super::general::{heading, setup_wizard};
use super::SettingsAction;
use crate::components::tool_rows;
use crate::theme::{settings, size, tokens};
use crate::tool_check::ToolCheck;
use crate::widgets::{button, ButtonVariant};
use eframe::egui::{RichText, Ui};

/// The daemon's answer, checked when the tab opens; `None` until the app has asked.
pub struct ExtrasPane<'a> {
    pub check: Option<&'a ToolCheck>,
}

pub(super) fn extras(ui: &mut Ui, pane: &ExtrasPane<'_>) -> Option<SettingsAction> {
    let p = tokens(ui.visuals().dark_mode);
    let mut action = None;
    let check = pane.check?;
    check.note_ui(ui);
    let list = check.list();
    if list.tools.is_empty() && check.is_checking() {
        ui.label(
            RichText::new("Checking the tools…")
                .size(size::SMALL)
                .color(p.text_low),
        );
        return None;
    }
    let optional = list.tools.iter().filter(|t| !t.required);
    if optional.clone().next().is_some() {
        heading(ui, "Extras");
        let can_install = check.can_install();
        if let Some((tool, a)) = tool_rows(ui, optional, check.installing(), |_| can_install) {
            action = Some(SettingsAction::Tool(tool, a));
        }
    } else {
        heading(ui, "Required tools");
        let required = list.tools.iter().filter(|t| t.required);
        // Installing them is the wizard's.
        if let Some((tool, a)) = tool_rows(ui, required, None, |_| false) {
            action = Some(SettingsAction::Tool(tool, a));
        }
        ui.add_space(8.0);
        if setup_wizard(ui) {
            action = Some(SettingsAction::RelaunchWizard);
        }
    }
    ui.add_space(settings::EXTRAS_GAP);
    if button(ui, "Check again", ButtonVariant::Default).clicked() {
        action = Some(SettingsAction::RecheckTools);
    }
    action
}
