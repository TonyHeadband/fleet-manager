//! Board, columns and cards, drawn from the daemon's `UiSchema`. Nothing here names a status, column or action.
mod archive_dialog;
mod board;
mod breakdown;
mod card;
mod close_dialog;
mod column;
mod command_hint;
mod fleet_row;
mod folder_picker;
mod footer;
mod new_task;
mod passport;
mod persona_filter;
mod persona_setup;
mod preview;
mod product_form;
mod release_filter;
mod reply;
mod repo_filter;
mod repo_picker;
mod repo_request;
mod requirements;
mod roster;
mod scanner;
mod tool_error;

pub use archive_dialog::{
    archive_dialog, confirm_dialog, delete_persona_dialog, ArchiveDialog, Confirm, ConfirmDialog,
    DeletePersonaDialog, PlanView,
};
pub use board::board;
pub use breakdown::{breakdown_panel, breakdown_panel_deciding, pending, pr_url, BreakdownAction};
pub use card::{card, fleet_card, task_card, CardState, CARD_HEIGHT};
pub use close_dialog::{close_dialog, DialogResult};
pub use column::column;
pub use command_hint::command_hint;
pub use fleet_row::{fleet_row, specialist_icon, FLEET_ROW_HEIGHT};
pub use folder_picker::{browse, FolderPicker, PickMode, Scan};
pub use footer::{footer, FooterAction};
pub use new_task::{new_task_dialog, workflow_line, SpawnAction, SpawnState};
pub(crate) use passport::ago;
pub use passport::{orphan_passport, task_passport, PassportAction, TaskPassport};
pub use persona_filter::{
    login_broken, persona_choice, persona_filter, persona_label, persona_labels,
};
pub use persona_setup::{
    caption, login_status, pat_field_id, pat_input, persona_setup_ui, Login, PersonaSetup,
    SetupAction, SetupMode, SetupStep,
};
pub use preview::{
    panel_layout, preview_pane, workspace_tip, AgentDetails, DiffView, FleetPanel, PanelLayout,
    PreviewAction, PreviewState, PreviewTab, Reply, RepoPanel, RequestDraft, TaskPanel,
};
pub use product_form::{
    product_fields, product_form, ProductForm, ProductFormAction, ProductSave, RepoDraft,
};
pub use release_filter::{release_filter, ReleaseFilter};
pub use reply::reply_box;
pub use repo_filter::{filter_summary, repo_filter};
pub use repo_picker::{repo_picker, PickerState};
pub use repo_request::{repo_request_box, RequestDecision};
pub use requirements::{requirements_ui, RequirementsAction};
pub use roster::{roster_chips, usable};
pub use scanner::Scanner;
pub use tool_error::{tool_error_ui, ToolAction};

use crate::theme::{radius, tokens};
use eframe::egui::{self, Align, Frame, Layout, Margin, Ui};
use fleet_api::{ActionDef, Agent};

/// The frame the shell's modals share (the same surface, radius and margin as the New agent dialog).
pub(crate) fn modal_frame(ctx: &egui::Context) -> Frame {
    let p = tokens(ctx.global_style().visuals.dark_mode);
    Frame::popup(&ctx.global_style())
        .fill(p.surface)
        .corner_radius(radius::DIALOG)
        .inner_margin(Margin::same(16))
}

/// A `surface` card in the middle of the window, for the screens before the board is drawn.
pub(crate) fn centred_card(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui)) {
    let p = tokens(ui.visuals().dark_mode);
    egui::CentralPanel::default().show(ui, |ui| {
        ui.add_space(ui.available_height() * 0.25);
        ui.with_layout(Layout::top_down(Align::Center), |ui| {
            Frame::new()
                .fill(p.surface)
                .corner_radius(radius::DIALOG)
                .inner_margin(Margin::same(16))
                .show(ui, |ui| {
                    ui.set_max_width(520.0);
                    add_contents(ui);
                });
        });
    });
}

/// What a drawn component asks the app to do. Components never touch the network themselves.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    // The agent is boxed: inline it would make every `Open` as large as a whole agent.
    Run {
        action: ActionDef,
        agent: Box<Agent>,
        input: Option<String>,
    },
    /// Open the preview for this agent id.
    Open(String),
    /// Show this stopped agent's task in the Tasks view.
    Details(String),
    /// Stop every running agent of this task id, after a confirm naming how many `fleet` agents run.
    StopTask { task: String, fleet: usize },
    /// Start a fresh orchestrator on this task id.
    RestartOrchestrator(String),
}
