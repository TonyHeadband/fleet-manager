use crate::theme::{radius, sans, size, tokens, tone_color, tone_tint, Weight};
use eframe::egui::{self, Margin, Response, RichText, Ui};
use fleet_api::Tone;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadgeTone {
    Neutral,
    Attention,
}

/// A small filled label. `tooltip` names what it shows (the schema field's label).
pub fn badge(ui: &mut Ui, text: &str, tooltip: &str, tone: BadgeTone) -> Response {
    let dark = ui.visuals().dark_mode;
    let p = tokens(dark);
    let (fill, colour, weight) = match tone {
        BadgeTone::Neutral => (p.raised, p.text, Weight::Regular),
        BadgeTone::Attention => (tone_tint(Tone::Attention, dark), tone_color(Tone::Attention, dark), Weight::Medium),
    };
    // Badges are smaller than card radii on purpose: they read as tags, not as nested cards.
    egui::Frame::new()
        .fill(fill)
        .corner_radius(radius::CONTROL)
        .inner_margin(Margin::symmetric(6, 1))
        .show(ui, |ui| ui.label(RichText::new(text).font(sans(ui, size::META + 0.5, weight)).color(colour)))
        .response
        .on_hover_text(tooltip)
}
