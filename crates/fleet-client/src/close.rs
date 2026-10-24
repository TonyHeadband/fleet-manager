//! What closing the window should do about the daemon, as pure decisions the UI only has to draw.
use crate::config::CloseChoice;
use fleet_api::{Agent, Status};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    /// Queued, working or waiting for the user: agents whose next event the user would miss.
    pub running: usize,
    pub working: usize,
    pub needs_you: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    CloseOnly,
    StopDaemonAndClose,
    Ask(Summary),
}

fn summarize(agents: &[Agent]) -> Summary {
    let count = |s: Status| agents.iter().filter(|a| a.status == s).count();
    let (queued, working, needs_you) = (count(Status::Queued), count(Status::Working), count(Status::NeedsYou));
    Summary { running: queued + working + needs_you, working, needs_you }
}

/// `we_started_daemon` is true only when this client launched the daemon it is talking to.
pub fn close_action(agents: &[Agent], we_started_daemon: bool, remembered: Option<CloseChoice>) -> CloseAction {
    match remembered {
        Some(CloseChoice::KeepRunning) => CloseAction::CloseOnly,
        // A remembered "stop" covers only a daemon this client started; one started by hand, or by another
        // window, is never stopped without the dialog.
        Some(CloseChoice::StopDaemon) if we_started_daemon => CloseAction::StopDaemonAndClose,
        // Ask always asks, even with nothing at risk: Settings offers it as a choice of its own, so stopping
        // without a word would go against what the user picked.
        Some(CloseChoice::StopDaemon) | None => CloseAction::Ask(summarize(agents)),
    }
}

pub fn dialog_text(s: &Summary) -> String {
    let consequence = "If you stop the daemon, the agents keep running in tmux, but their requests for you (Needs you), \
their finished work and their failures will not be reported. A blocked agent just waits. On the next start the \
running agents show \"status unverified since daemon restart\" until their next event. Cards, branches and worktrees are kept.";
    if s.running == 0 {
        return "No agents are running, so stopping the daemon misses nothing. Cards, branches and worktrees are kept.".to_string();
    }
    let mut parts = Vec::new();
    if s.working > 0 {
        parts.push(format!("{} working", s.working));
    }
    if s.needs_you > 0 {
        parts.push(format!("{} waiting for you", s.needs_you));
    }
    let lead = if s.running == 1 { "1 agent is running".to_string() } else { format!("{} agents are running", s.running) };
    let detail = if parts.is_empty() { String::new() } else { format!(": {}", parts.join(", ")) };
    format!("{lead}{detail}. {consequence}")
}
