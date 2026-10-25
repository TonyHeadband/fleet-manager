use crate::theme::{radius, sans, size, tokens, tone_color, tone_tint, Weight};
use eframe::egui::{self, Color32, Margin, Response, RichText, Ui};
use fleet_api::Tone;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadgeTone {
    Neutral,
    Attention,
    /// Tinted with a schema tone (a stop cause's badge).
    Tinted(Tone),
}

/// Horizontal padding inside the badge's frame, each side.
const PAD_X: i8 = 6;

fn style(ui: &Ui, tone: BadgeTone) -> (Color32, Color32, Weight) {
    let dark = ui.visuals().dark_mode;
    let tinted = |t: Tone| (tone_tint(t, dark), tone_color(t, dark), Weight::Medium);
    match tone {
        BadgeTone::Neutral => (tokens(dark).raised, tokens(dark).text, Weight::Regular),
        BadgeTone::Attention => tinted(Tone::Attention),
        BadgeTone::Tinted(t) => tinted(t),
    }
}

fn font(ui: &Ui, weight: Weight) -> egui::FontId {
    sans(ui, size::META + 0.5, weight)
}

/// A small filled label. `tooltip` names what it shows (the schema field's label).
pub fn badge(ui: &mut Ui, text: &str, tooltip: &str, tone: BadgeTone) -> Response {
    let (fill, colour, weight) = style(ui, tone);
    // Badges are smaller than card radii on purpose: they read as tags, not as nested cards.
    egui::Frame::new()
        .fill(fill)
        .corner_radius(radius::CONTROL)
        .inner_margin(Margin::symmetric(PAD_X, 1))
        .show(ui, |ui| {
            ui.label(RichText::new(text).font(font(ui, weight)).color(colour))
        })
        .response
        .on_hover_text(tooltip)
}
