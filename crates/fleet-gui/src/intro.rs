//! The screen the window opens on: the logo, turning, above a bar that names the startup step under way.
use crate::theme::{self, tokens};
use crate::widgets::fui;
use eframe::egui::{self, Align, Layout, RichText, Sense, Ui, Vec2};

/// How far startup is: `done` of `total` steps, and the one under way.
#[derive(Debug, Clone, PartialEq)]
pub struct IntroProgress {
    pub label: String,
    pub done: usize,
    pub total: usize,
}

const LOGO: f32 = 160.0;
/// Seconds per turn of the logo's radar.
const TURN: f32 = 4.0;
const BAR_WIDTH: f32 = 320.0;

/// The intro. The logo turns only while animations are on; off, it is drawn still and asks for no frames, so the
/// screen repaints when startup moves on.
pub fn intro_ui(ui: &mut Ui, progress: IntroProgress) {
    let p = tokens(ui.visuals().dark_mode);
    egui::CentralPanel::default().show(ui, |ui| {
        ui.add_space(ui.available_height() * 0.2);
        ui.with_layout(Layout::top_down(Align::Center), |ui| {
            let angle = theme::spin(ui, TURN).unwrap_or(0.0) * std::f32::consts::TAU;
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(LOGO), Sense::hover());
            fui::radar_mark(ui.painter(), rect.center(), LOGO, p.fill, angle);
            ui.add_space(24.0);
            let fraction = if progress.total == 0 {
                0.0
            } else {
                progress.done as f32 / progress.total as f32
            };
            ui.add(egui::ProgressBar::new(fraction.min(1.0)).desired_width(BAR_WIDTH));
            ui.add_space(8.0);
            ui.label(RichText::new(progress.label).color(p.text_low));
        });
    });
}
