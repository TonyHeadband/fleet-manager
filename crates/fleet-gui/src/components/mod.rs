//! Board, columns and cards, drawn from the daemon's `UiSchema`. Nothing here names a status, column or action.
mod archive_dialog;
mod board;
mod card;
mod close_dialog;
mod column;
mod command_hint;
mod folder_picker;
mod footer;
mod new_agent;
mod passport;
mod persona_filter;
mod persona_setup;
mod preview;
mod reply;
mod repo_filter;
mod repo_picker;
mod repo_request;
mod scanner;

pub use archive_dialog::{
    archive_dialog, delete_persona_dialog, end_session_dialog, ArchiveDialog, Confirm,
    DeletePersonaDialog, EndSessionDialog, PlanView,
};
pub use board::board;
pub use card::{card, CardState, CARD_HEIGHT};
pub use close_dialog::{close_dialog, DialogResult};
pub use column::column;
pub use command_hint::command_hint;
pub use folder_picker::{browse, FolderPicker, PickMode, Scan};
pub use footer::{footer, FooterAction};
pub use new_agent::{new_agent_dialog, SpawnAction, SpawnState};
pub use passport::{orphan_passport, project_passport, PassportAction, ProjectPassport};
pub use persona_filter::{
    login_broken, persona_choice, persona_filter, persona_label, persona_labels,
};
pub use persona_setup::{
    caption, login_status, pat_field_id, pat_input, persona_setup_ui, Login, PersonaSetup,
    SetupAction, SetupMode, SetupStep,
};
pub use preview::{
    preview_pane, workspace_tip, AgentDetails, DiffView, PreviewAction, PreviewState, PreviewTab,
    Reply, RepoPanel, RequestDraft,
};
pub use reply::reply_box;
pub use repo_filter::{filter_summary, repo_filter};
pub use repo_picker::{repo_picker, PickerState};
pub use repo_request::{repo_request_box, RequestDecision};
pub use scanner::Scanner;

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
    /// Show this stopped agent's project in the Projects view.
    Details(String),
}
