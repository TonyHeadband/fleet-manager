//! Settings: the app's and the daemon's options, one tab per area.
mod daemon;
mod general;
mod log;

pub use daemon::DaemonRows;
pub use general::{GeneralChange, ProjectsFolder};
pub use log::{tail, LogFeed, LogView, Source, TAIL};

use super::{title, View};
use crate::theme::{size, tokens};
use crate::widgets::tab_bar;
use eframe::egui::{self, Frame, Margin, RichText, Ui};
use fleet_client::config::{CloseChoice, ThemeChoice};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SettingsTab {
    General,
    Daemon,
    Log,
}

const TABS: [(SettingsTab, &str); 3] = [(SettingsTab::General, "General"), (SettingsTab::Daemon, "Daemon"), (SettingsTab::Log, "Log")];

/// What the tabs show. `projects` and `daemon` are `None` for an app that cannot restart the daemon.
pub struct SettingsState<'a> {
    pub theme: ThemeChoice,
    pub close_choice: Option<CloseChoice>,
    pub sticky_alerts: bool,
    pub projects: Option<ProjectsFolder<'a>>,
    pub daemon: Option<DaemonRows<'a>>,
    pub log: LogPane<'a>,
}

/// The Log tab's own state and what it lists.
pub struct LogPane<'a> {
    pub view: &'a mut LogView,
    pub feed: LogFeed<'a>,
}

/// What the user asked for in Settings; the app carries it out.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SettingsAction {
    Change(GeneralChange),
    SaveProjectsRoot,
    RestartDaemon,
}

pub(crate) fn header(ui: &mut Ui) {
    title(ui, View::Settings);
}

/// The tab bar and the selected tab. `tab` is the app's, so the last tab is kept while the app runs.
pub(crate) fn main(ui: &mut Ui, tab: &mut SettingsTab, state: SettingsState<'_>) -> Option<SettingsAction> {
    let p = tokens(ui.visuals().dark_mode);
    let mut action = None;
    egui::CentralPanel::default().frame(Frame::new().fill(p.bg)).show(ui, |ui| {
        if let Some(picked) = tab_bar(ui, &TABS, *tab) {
            *tab = picked;
        }
        Frame::new().inner_margin(Margin::symmetric(22, 18)).show(ui, |ui| match *tab {
            SettingsTab::General => action = general::general(ui, state),
            SettingsTab::Daemon => match &state.daemon {
                Some(rows) => action = daemon::daemon(ui, rows),
                None => nothing_yet(ui),
            },
            SettingsTab::Log => state.log.view.show(ui, state.log.feed),
        });
    });
    action
}

fn nothing_yet(ui: &mut Ui) {
    let p = tokens(ui.visuals().dark_mode);
    ui.label(RichText::new("Nothing here yet.").size(size::SMALL).color(p.text_low));
}
