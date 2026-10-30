//! egui's close flow (`fleet-gui/src/shell.rs`, `CloseFlow`) for the Tauri window: the decision is fleet-client's
//! `close_action`; the web draws the dialog and answers through `answer_close`.
use crate::bridge::{Owner, Paths};
use fleet_client::close::{close_action, CloseAction};
use fleet_client::config::{CloseChoice, SettingsWriter};
use fleet_client::daemon_control::{pidfile_names, stop_daemon};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    Idle,
    Asking,
    Stopping,
    StopFailed,
}

/// What the window does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Close,
    Stop,
    Ask,
    Ignore,
}

#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Answer {
    Keep,
    Stop,
    Cancel,
    CloseAnyway,
}

#[derive(Debug, Default)]
pub struct Flow {
    pub phase: Phase,
    /// The board is up. Before it (the boot view, the wizard) a close just closes, as egui's window does.
    pub armed: bool,
}

impl Flow {
    /// The user asked to close the window. While a dialog is up or a stop runs, a second request changes nothing.
    pub fn request(&mut self, still_ours: bool, remembered: Option<CloseChoice>) -> Decision {
        if !self.armed {
            return Decision::Close;
        }
        if self.phase != Phase::Idle {
            return Decision::Ignore;
        }
        // The agents only feed the dialog's numbers, which the web counts live.
        match close_action(&[], still_ours, remembered) {
            CloseAction::CloseOnly => Decision::Close,
            CloseAction::StopDaemonAndClose => {
                self.phase = Phase::Stopping;
                Decision::Stop
            }
            CloseAction::Ask(_) => {
                self.phase = Phase::Asking;
                Decision::Ask
            }
        }
    }

    pub fn answer(&mut self, answer: Answer) -> Decision {
        match (self.phase, answer) {
            (Phase::Asking, Answer::Keep) | (Phase::StopFailed, Answer::CloseAnyway) => {
                self.phase = Phase::Idle;
                Decision::Close
            }
            (Phase::Asking, Answer::Stop) => {
                self.phase = Phase::Stopping;
                Decision::Stop
            }
            (Phase::Asking | Phase::StopFailed, Answer::Cancel) => {
                self.phase = Phase::Idle;
                Decision::Ignore
            }
            _ => Decision::Ignore,
        }
    }

    pub fn stopped(&mut self, ok: bool) -> Decision {
        if self.phase != Phase::Stopping {
            return Decision::Ignore;
        }
        if ok {
            self.phase = Phase::Idle;
            return Decision::Close;
        }
        self.phase = Phase::StopFailed;
        Decision::Ignore
    }
}

#[derive(Default)]
pub struct CloseState(Mutex<Flow>);

impl CloseState {
    fn with<T>(&self, f: impl FnOnce(&mut Flow) -> T) -> T {
        f(&mut self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

/// What the web is told: the dialog to show, the stop under way, or why it failed.
#[derive(Serialize, Clone, Debug)]
#[serde(tag = "phase", rename_all = "kebab-case")]
pub enum CloseEvent {
    Asking,
    Stopping,
    StopFailed { message: String },
}

/// Started by this window and not replaced since: someone may have restarted it by hand, and that daemon is not
/// this window's to stop without asking.
fn still_ours(app: &AppHandle) -> bool {
    let state_dir = app.state::<Paths>().load().state_dir;
    app.state::<Owner>()
        .pid()
        .is_some_and(|pid| pidfile_names(&state_dir, pid))
}

/// The window's close button. Returns whether the window may close now.
pub fn requested(app: &AppHandle) -> bool {
    // Settings changes the choice while the window is open; the file, with changes still being written, is the truth.
    let choice = SettingsWriter::shared()
        .current(&app.state::<Paths>().settings())
        .close_choice;
    let ours = still_ours(app);
    let decision = app.state::<CloseState>().with(|f| f.request(ours, choice));
    if decision == Decision::Close {
        SettingsWriter::shared().flush();
        return true;
    }
    act(app, decision);
    false
}

fn act(app: &AppHandle, decision: Decision) {
    match decision {
        Decision::Close => {
            // A choice made just before closing is kept.
            SettingsWriter::shared().flush();
            app.exit(0);
        }
        Decision::Ask => {
            let _ = app.emit("close", CloseEvent::Asking);
        }
        Decision::Stop => {
            let _ = app.emit("close", CloseEvent::Stopping);
            let app = app.clone();
            std::thread::spawn(move || {
                let cfg = app.state::<Paths>().load();
                let result =
                    stop_daemon(&cfg.state_dir, &cfg.base_url()).map_err(|e| e.to_string());
                let next = app
                    .state::<CloseState>()
                    .with(|f| f.stopped(result.is_ok()));
                match result {
                    Err(message) if next == Decision::Ignore => {
                        let _ = app.emit("close", CloseEvent::StopFailed { message });
                    }
                    _ => act(&app, next),
                }
            });
        }
        Decision::Ignore => {}
    }
}

#[tauri::command]
pub fn answer_close(app: AppHandle, answer: Answer, remember: bool) {
    let choice = match answer {
        Answer::Keep => Some(CloseChoice::KeepRunning),
        Answer::Stop => Some(CloseChoice::StopDaemon),
        _ => None,
    };
    if let Some(choice) = choice.filter(|_| remember) {
        // Through the writer, as Settings writes the same file; a failed save only costs the next close a question.
        SettingsWriter::shared().update(
            &app.state::<Paths>().settings(),
            move |s| s.close_choice = Some(choice),
            |_| {},
        );
    }
    let decision = app.state::<CloseState>().with(|f| f.answer(answer));
    act(&app, decision);
}

/// The board is up (or gone again): only then does a close go through the flow.
#[tauri::command]
pub fn set_close_armed(app: AppHandle, armed: bool) {
    app.state::<CloseState>().with(|f| f.armed = armed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use fleet_client::config::CloseChoice;

    fn armed() -> Flow {
        Flow {
            armed: true,
            ..Flow::default()
        }
    }

    #[test]
    fn before_the_board_a_close_just_closes() {
        assert_eq!(Flow::default().request(true, None), Decision::Close);
    }

    #[test]
    fn a_remembered_stop_stops_a_daemon_still_ours_silently() {
        let mut flow = armed();
        assert_eq!(
            flow.request(true, Some(CloseChoice::StopDaemon)),
            Decision::Stop
        );
        assert_eq!(flow.phase, Phase::Stopping);
    }

    #[test]
    fn a_remembered_stop_asks_about_a_daemon_not_ours() {
        let mut flow = armed();
        assert_eq!(
            flow.request(false, Some(CloseChoice::StopDaemon)),
            Decision::Ask
        );
        assert_eq!(flow.phase, Phase::Asking);
    }

    #[test]
    fn a_remembered_keep_closes_and_no_choice_asks() {
        assert_eq!(
            armed().request(true, Some(CloseChoice::KeepRunning)),
            Decision::Close
        );
        assert_eq!(armed().request(true, None), Decision::Ask);
    }

    #[test]
    fn a_second_close_request_while_asking_or_stopping_changes_nothing() {
        let mut flow = armed();
        flow.request(true, None);
        assert_eq!(
            flow.request(true, Some(CloseChoice::StopDaemon)),
            Decision::Ignore
        );
        assert_eq!(flow.phase, Phase::Asking);
        flow.answer(Answer::Stop);
        assert_eq!(flow.request(true, None), Decision::Ignore);
        assert_eq!(flow.phase, Phase::Stopping);
    }

    #[test]
    fn answers_move_the_flow_as_egui_s() {
        let mut flow = armed();
        flow.request(false, None);
        assert_eq!(flow.answer(Answer::Cancel), Decision::Ignore);
        assert_eq!(flow.phase, Phase::Idle);
        flow.request(false, None);
        assert_eq!(flow.answer(Answer::Keep), Decision::Close);
        let mut flow = armed();
        flow.request(false, None);
        assert_eq!(flow.answer(Answer::Stop), Decision::Stop);
        // An answer with nothing asked (a stale click) does nothing.
        assert_eq!(armed().answer(Answer::Stop), Decision::Ignore);
    }

    #[test]
    fn a_failed_stop_waits_for_close_anyway_or_cancel() {
        let mut flow = armed();
        flow.request(true, Some(CloseChoice::StopDaemon));
        flow.stopped(false);
        assert_eq!(flow.phase, Phase::StopFailed);
        assert_eq!(flow.request(true, None), Decision::Ignore);
        assert_eq!(flow.answer(Answer::Cancel), Decision::Ignore);
        assert_eq!(flow.phase, Phase::Idle);
    }
}
