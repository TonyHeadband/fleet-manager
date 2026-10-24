//! Board, columns and cards, drawn from the daemon's `UiSchema`. Nothing here names a status, column or action.
mod board;
mod card;
mod close_dialog;
mod column;
mod preview;
mod repo_filter;
mod spawn_form;

pub use board::board;
pub use card::{card, CardState};
pub use close_dialog::{close_dialog, DialogResult};
pub use column::column;
pub use preview::{preview_pane, DiffView, PreviewAction, PreviewState, PreviewTab};
pub use repo_filter::repo_filter;
pub use spawn_form::{spawn_form, SpawnState};

use fleet_api::{ActionDef, Agent};

/// What a drawn component asks the app to do. Components never touch the network themselves.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Run { action: ActionDef, agent: Agent, input: Option<String> },
    /// Open the preview for this agent id.
    Open(String),
}
