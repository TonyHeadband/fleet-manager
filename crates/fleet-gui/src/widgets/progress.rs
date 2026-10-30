use crate::theme::{radius, releases, tokens, tone_color};
use eframe::egui::{Sense, Ui, Vec2};
use fleet_api::Tone;

/// A thin bar filled to `fraction` (clamped to 0..=1) in `tone`'s colour, on the `raised` track.
pub fn progress_bar(ui: &mut Ui, fraction: f32, tone: Tone) {
    let dark = ui.visuals().dark_mode;
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(releases::BAR_WIDTH, releases::BAR_HEIGHT),
        Sense::hover(),
    );
    ui.painter()
        .rect_filled(rect, radius::CONTROL, tokens(dark).raised);
    let mut filled = rect;
    filled.set_width(rect.width() * fraction.clamp(0.0, 1.0));
    if filled.width() > 0.0 {
        ui.painter()
            .rect_filled(filled, radius::CONTROL, tone_color(tone, dark));
    }
}
