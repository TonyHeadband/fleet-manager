//! Board, columns and cards, drawn from the daemon's `UiSchema`. Nothing here names a status, column or action.
mod board;
mod card;
mod close_dialog;
mod column;
mod folder_picker;
mod footer;
mod new_agent;
mod preview;
mod repo_filter;
mod repo_picker;
mod repo_request;
mod reply;
mod scanner;
mod trust_hint;

pub use board::board;
pub use card::{card, CardState};
pub use close_dialog::{close_dialog, DialogResult};
pub use column::column;
pub use footer::{footer, FooterAction};
pub use folder_picker::{browse, FolderPicker, PickMode, Scan};
pub use new_agent::{new_agent_dialog, SpawnState};
pub use preview::{preview_pane, workspace_tip, DiffView, PreviewAction, PreviewState, PreviewTab, RepoPanel, Reply, RequestDraft};
pub use repo_filter::{filter_summary, repo_filter};
pub use repo_picker::{repo_picker, PickerState};
pub use repo_request::{repo_request_box, RequestDecision};
pub use reply::reply_box;
pub use scanner::Scanner;
pub use trust_hint::trust_hint;

use crate::theme::{radius, tokens};
use eframe::egui::{self, Align, Frame, Layout, Margin, Ui};
use fleet_api::{ActionDef, Agent};

/// The frame the shell's modals share (the same surface, radius and margin as the New agent dialog).
pub(crate) fn modal_frame(ctx: &egui::Context) -> Frame {
    let p = tokens(ctx.global_style().visuals.dark_mode);
    Frame::popup(&ctx.global_style()).fill(p.surface).corner_radius(radius::DIALOG).inner_margin(Margin::same(16))
}

/// A `surface` card in the middle of the window, for the screens before the board is drawn.
pub(crate) fn centred_card(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui)) {
    let p = tokens(ui.visuals().dark_mode);
    egui::CentralPanel::default().show(ui, |ui| {
        ui.add_space(ui.available_height() * 0.25);
        ui.with_layout(Layout::top_down(Align::Center), |ui| {
            Frame::new().fill(p.surface).corner_radius(radius::DIALOG).inner_margin(Margin::same(16)).show(ui, |ui| {
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
    Run { action: ActionDef, agent: Box<Agent>, input: Option<String> },
    /// Open the preview for this agent id.
    Open(String),
}
