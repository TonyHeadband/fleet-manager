//! Settings: the app's and the daemon's options, one tab per area.
mod daemon;
mod extras;
mod general;
mod log;

pub use daemon::DaemonRows;
pub use extras::ExtrasPane;
pub use general::{FolderField, GeneralChange, LimitsField, PendingFolders};
pub use log::{tail, LogFeed, LogView, Source, TAIL};

use super::{title, View};
use crate::components::ToolAction;
use crate::theme::{size, tokens, Fonts};
use crate::widgets::tab_bar;
use eframe::egui::{self, Frame, Margin, RichText, Ui};
use fleet_api::ToolId;
use fleet_client::config::{CloseChoice, ThemeChoice};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SettingsTab {
    General,
    Daemon,
    Extras,
    Log,
}

const TABS: [(SettingsTab, &str); 4] = [
    (SettingsTab::General, "General"),
    (SettingsTab::Daemon, "Daemon"),
    (SettingsTab::Extras, "Extras"),
    (SettingsTab::Log, "Log"),
];

/// What the tabs show. `tasks`, `personas`, `limits` and `daemon` are `None` for an app that cannot restart the daemon.
pub struct SettingsState<'a> {
    pub theme: ThemeChoice,
    pub close_choice: Option<CloseChoice>,
    pub sticky_alerts: bool,
    pub fonts: Fonts,
    pub animations: bool,
    pub tasks: Option<FolderField<'a>>,
    pub personas: Option<FolderField<'a>>,
    pub limits: Option<LimitsField<'a>>,
    pub daemon: Option<DaemonRows<'a>>,
    /// Folders written to the config that wait for the daemon's restart.
    pub pending: Option<PendingFolders>,
    pub extras: ExtrasPane<'a>,
    pub log: LogPane<'a>,
}

/// The Log tab's own state and what it lists.
pub struct LogPane<'a> {
    pub view: &'a mut LogView,
    pub feed: LogFeed<'a>,
}

/// What the user asked for in Settings; the app carries it out.
#[derive(Clone, PartialEq, Debug)]
pub enum SettingsAction {
    Change(GeneralChange),
    SaveTasksRoot,
    SavePersonasDir,
    SaveLimits,
    RestartDaemon,
    /// Run the setup wizard again, from its Requirements step, over the app.
    RelaunchWizard,
    /// A tool's row on Extras asked for this.
    Tool(ToolId, ToolAction),
    /// Check again on Extras.
    RecheckTools,
}

pub(crate) fn header(ui: &mut Ui) {
    title(ui, View::Settings);
}

/// The tab bar and the selected tab. `tab` is the app's, so the last tab is kept while the app runs.
pub(crate) fn main(
    ui: &mut Ui,
    tab: &mut SettingsTab,
    state: SettingsState<'_>,
) -> Option<SettingsAction> {
    let p = tokens(ui.visuals().dark_mode);
    let mut action = None;
    egui::CentralPanel::default()
        .frame(Frame::new().fill(p.bg))
        .show(ui, |ui| {
            if let Some(picked) = tab_bar(ui, &TABS, *tab) {
                *tab = picked;
                // Drawn again at once, so the app can prepare the new tab (Extras' check) before its first frame.
                ui.ctx().request_discard("Settings tab changed");
            }
            Frame::new()
                .inner_margin(Margin::symmetric(22, 18))
                .show(ui, |ui| match *tab {
                    // Long enough to pass the bottom of a small window.
                    SettingsTab::General => {
                        egui::ScrollArea::vertical()
                            .show(ui, |ui| action = general::general(ui, state));
                    }
                    SettingsTab::Daemon => match &state.daemon {
                        Some(rows) => action = daemon::daemon(ui, rows),
                        None => nothing_yet(ui),
                    },
                    SettingsTab::Extras => action = extras::extras(ui, &state.extras),
                    SettingsTab::Log => state.log.view.show(ui, state.log.feed),
                });
        });
    action
}

fn nothing_yet(ui: &mut Ui) {
    let p = tokens(ui.visuals().dark_mode);
    ui.label(
        RichText::new("Nothing here yet.")
            .size(size::SMALL)
            .color(p.text_low),
    );
}
