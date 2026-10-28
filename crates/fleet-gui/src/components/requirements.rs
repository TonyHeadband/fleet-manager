//! The wizard's Requirements step: what the daemon found of each tool, and the way out for what it did not.
use super::tool_error::{tool_error_ui, ToolAction};
use crate::theme::{size, tokens};
use crate::widgets::{button, ButtonVariant};
use eframe::egui::{self, RichText, Ui};
use fleet_api::{ToolId, ToolList, ToolStatus};

/// What the user asked for on the Requirements step; the shell runs it.
#[derive(Debug, Clone, PartialEq)]
pub enum RequirementsAction {
    Tool(ToolId, ToolAction),
    Recheck,
    Continue,
}

/// One row per tool in `list`, then Check again and Continue. `installing` is the tool whose install terminal is
/// open: no other install starts meanwhile. `can_install` says whether the app may run a tool's command itself.
pub fn requirements_ui(
    ui: &mut Ui,
    list: &ToolList,
    installing: Option<ToolId>,
    can_install: impl Fn(&ToolStatus) -> bool,
) -> Option<RequirementsAction> {
    let p = tokens(ui.visuals().dark_mode);
    let mut action = None;
    for status in &list.tools {
        let name = status.tool.name();
        match &status.error {
            None => {
                let text = match &status.version {
                    Some(v) => format!("✓ {name} {v}"),
                    None => format!("✓ {name}"),
                };
                ui.label(RichText::new(text).color(p.text));
            }
            Some(e) => {
                let allowed = installing.is_none() && can_install(status);
                if let Some(a) = tool_error_ui(ui, e, allowed) {
                    action = Some(RequirementsAction::Tool(status.tool, a));
                }
                if installing == Some(status.tool) {
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!(
                                "Installing {name} in its own window; it is checked again when that window closes."
                            ))
                            .size(size::SMALL)
                            .color(p.text_low),
                        )
                        .wrap(),
                    );
                }
            }
        }
        ui.add_space(8.0);
    }
    let missing = list.missing_required().next().is_some();
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if button(ui, "Check again", ButtonVariant::Default).clicked() {
            action = Some(RequirementsAction::Recheck);
        }
        let label = if missing {
            "Continue without it"
        } else {
            "Continue"
        };
        if button(ui, label, ButtonVariant::Primary).clicked() {
            action = Some(RequirementsAction::Continue);
        }
    });
    action
}
