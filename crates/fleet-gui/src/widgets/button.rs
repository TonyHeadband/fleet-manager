use crate::theme::palette;
use eframe::egui::{self, Color32, Response, RichText, Ui};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonVariant {
    Default,
    Danger,
    QuietDanger,
}

/// The one button. Wrap it in `ui.add_enabled_ui(false, …)` for a disabled one.
pub fn button(ui: &mut Ui, label: &str, variant: ButtonVariant) -> Response {
    let p = palette(ui.visuals().dark_mode);
    let b = match variant {
        ButtonVariant::Default => egui::Button::new(label),
        ButtonVariant::Danger => egui::Button::new(RichText::new(label).color(Color32::WHITE)).fill(p.danger),
        ButtonVariant::QuietDanger => egui::Button::new(RichText::new(label).color(p.danger)),
    };
    ui.add(b)
}

/// A button that stays highlighted while `pressed` (filter chips, the selected tab).
pub fn toggle(ui: &mut Ui, label: &str, pressed: bool) -> Response {
    ui.add(egui::Button::new(label).selected(pressed))
}
