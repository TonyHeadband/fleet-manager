use crate::theme::palette;
use eframe::egui::{self, Color32, Margin, Response, RichText, Stroke, Ui};

/// A small outlined label. `tooltip` names what it shows (the schema field's label).
pub fn badge(ui: &mut Ui, text: &str, tooltip: &str) -> Response {
    badge_accent(ui, text, tooltip, None)
}

/// Like [`badge`], with an accent colour for the outline and text (the blocked-reason badge uses one).
pub fn badge_accent(ui: &mut Ui, text: &str, tooltip: &str, accent: Option<Color32>) -> Response {
    let p = palette(ui.visuals().dark_mode);
    let colour = accent.unwrap_or(p.text);
    egui::Frame::new()
        .stroke(Stroke::new(1.0, accent.unwrap_or(p.border)))
        .corner_radius(10)
        .inner_margin(Margin::symmetric(8, 1))
        .show(ui, |ui| ui.label(RichText::new(text).small().color(colour)))
        .response
        .on_hover_text(tooltip)
}
