//! Settings → Daemon: what the window knows about the daemon, and Restart daemon.
use super::general::heading;
use super::SettingsAction;
use crate::shell::DaemonCtl;
use crate::theme::{mono, size, tokens};
use crate::widgets::{button, ButtonVariant};
use eframe::egui::{self, RichText, Ui};

pub struct DaemonRows<'a> {
    pub ctl: &'a DaemonCtl,
    /// Why Restart daemon cannot run now, if it cannot.
    pub blocked: Option<&'static str>,
    pub restarting: bool,
}

pub(super) fn daemon(ui: &mut Ui, rows: &DaemonRows<'_>) -> Option<SettingsAction> {
    let p = tokens(ui.visuals().dark_mode);
    let d = rows.ctl;
    // Read on every frame the tab is drawn: the window is idle otherwise, and the file changes under it when the
    // daemon is restarted from here or by hand.
    let pid = std::fs::read_to_string(d.state_dir.join("daemon.pid")).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    let started_by = if d.we_started { "this window" } else { "already running" };
    let (state_dir, config) = (d.state_dir.display().to_string(), d.config_path.display().to_string());
    let mut lines: Vec<(&str, &str, bool)> = vec![("Address", &d.listen, true), ("Started by", started_by, false), ("State directory", &state_dir, true), ("Config file", &config, true)];
    if let Some(pid) = &pid {
        lines.push(("PID", pid, true));
    }
    heading(ui, "Daemon");
    egui::Grid::new("daemon_rows").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
        for (name, value, code) in lines {
            ui.label(RichText::new(name).color(p.text_low));
            let value = if code { RichText::new(value).font(mono(size::CODE)) } else { RichText::new(value) };
            ui.label(value.color(p.text));
            ui.end_row();
        }
    });
    ui.add_space(12.0);
    let mut restart = false;
    ui.horizontal(|ui| {
        let clicked = ui.add_enabled_ui(rows.blocked.is_none(), |ui| button(ui, "Restart daemon", ButtonVariant::Default)).inner;
        restart = clicked.on_disabled_hover_text(rows.blocked.unwrap_or_default()).clicked();
        if rows.restarting {
            ui.label(RichText::new("Restarting the daemon…").size(size::SMALL).color(p.text_low));
        }
    });
    restart.then_some(SettingsAction::RestartDaemon)
}
