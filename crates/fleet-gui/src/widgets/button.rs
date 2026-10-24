use crate::theme::{radius, sans, size, tokens, Weight};
use eframe::egui::{self, Color32, Response, RichText, Stroke, Ui, Vec2};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonVariant {
    Primary,
    Default,
    Ghost,
    Danger,
    QuietDanger,
}

const MIN_HEIGHT: f32 = 24.0;

/// The one button. Wrap it in `ui.add_enabled_ui(false, …)` for a disabled one.
pub fn button(ui: &mut Ui, label: &str, variant: ButtonVariant) -> Response {
    let dark = ui.visuals().dark_mode;
    let p = tokens(dark);
    let text = |colour: Color32, weight: Weight| RichText::new(label).font(sans(ui, size::BUTTON, weight)).color(colour);
    let b = match variant {
        ButtonVariant::Primary => egui::Button::new(text(p.on_fill, Weight::Medium)),
        // Hover and press come from the theme's widget visuals (raised fill, outline).
        ButtonVariant::Default => egui::Button::new(RichText::new(label).font(sans(ui, size::BUTTON, Weight::Regular))),
        ButtonVariant::Ghost => egui::Button::new(text(p.text_low, Weight::Regular)).frame_when_inactive(false),
        // `danger` is the same red-ish fill in both themes, so the label is the light theme's white.
        ButtonVariant::Danger => {
            let on_danger = if dark { tokens(false).surface } else { p.on_fill };
            egui::Button::new(text(on_danger, Weight::Regular))
        }
        ButtonVariant::QuietDanger => egui::Button::new(text(p.danger, Weight::Regular)).frame_when_inactive(false),
    }
    .corner_radius(radius::CONTROL)
    .min_size(Vec2::new(0.0, MIN_HEIGHT));

    if variant == ButtonVariant::Default {
        return ui.add(b);
    }
    // An explicit `Button::fill` would paint in every state, so the fills go through scoped visuals instead.
    ui.scope(|ui| {
        let w = &mut ui.visuals_mut().widgets;
        // Only the fill answers hover; the theme's hover outline would turn a flat button into a boxed one.
        w.hovered.bg_stroke = Stroke::NONE;
        w.active.bg_stroke = Stroke::NONE;
        if let Some([rest, hover, press]) = state_fills(variant, dark) {
            w.inactive.bg_stroke = Stroke::NONE;
            // A disabled button is drawn with the noninteractive visuals; it keeps its resting fill and egui fades it.
            for (state, fill) in [(&mut w.noninteractive, rest), (&mut w.inactive, rest), (&mut w.hovered, hover), (&mut w.active, press)] {
                state.weak_bg_fill = fill;
                state.bg_fill = fill;
            }
            w.noninteractive.bg_stroke = Stroke::NONE;
        }
        ui.add(b)
    })
    .inner
}

/// Resting, hovered and pressed fills of the variants that paint their own; `None` leaves the theme's.
/// Hover shades are mixed from tokens, so they follow the theme with no literal colour.
fn state_fills(variant: ButtonVariant, dark: bool) -> Option<[Color32; 3]> {
    let p = tokens(dark);
    match variant {
        ButtonVariant::Primary => Some([p.fill, p.text, p.text_low]),
        ButtonVariant::Danger => Some([p.danger, p.danger.lerp_to_gamma(p.text_high, 0.2), p.danger.lerp_to_gamma(p.text_high, 0.35)]),
        ButtonVariant::Default | ButtonVariant::Ghost | ButtonVariant::QuietDanger => None,
    }
}

/// A button that stays highlighted while `pressed` (filter chips).
pub fn toggle(ui: &mut Ui, label: &str, pressed: bool) -> Response {
    ui.add(egui::Button::new(label).selected(pressed))
}

/// A pick-one-or-many chip: the selected one sits on `raised`, which is visible on a `surface` dialog where the
/// selection tint would hide the label's weight. Reports itself as selected for screen readers and tests.
pub fn chip(ui: &mut Ui, label: &str, selected: bool) -> Response {
    let p = tokens(ui.visuals().dark_mode);
    let b = egui::Button::new(label).selected(selected);
    ui.add(if selected { b.fill(p.raised).stroke(egui::Stroke::new(1.0, p.text_low)) } else { b })
}
