use super::fui;
use crate::theme::{radius, sans, size, tokens, Weight};
use eframe::egui::{self, Color32, Response, RichText, Shape, Stroke, Ui, Vec2};

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
    let text = |colour: Color32, weight: Weight| {
        RichText::new(label)
            .font(sans(ui, size::BUTTON, weight))
            .color(colour)
    };
    let b = match variant {
        ButtonVariant::Primary => egui::Button::new(text(p.on_fill, Weight::Medium)),
        // Hover and press come from the theme's widget visuals (raised fill, outline).
        ButtonVariant::Default => {
            egui::Button::new(RichText::new(label).font(sans(ui, size::BUTTON, Weight::Regular)))
        }
        ButtonVariant::Ghost => {
            egui::Button::new(text(p.text_low, Weight::Regular)).frame_when_inactive(false)
        }
        // `on_fill` reads on `danger` too: navy on the light red of dark, white on the deep red of light.
        ButtonVariant::Danger => egui::Button::new(text(p.on_fill, Weight::Regular)),
        ButtonVariant::QuietDanger => {
            egui::Button::new(text(p.danger, Weight::Regular)).frame_when_inactive(false)
        }
    }
    .corner_radius(radius::CONTROL)
    .min_size(Vec2::new(0.0, MIN_HEIGHT));

    if variant == ButtonVariant::Default {
        return ui.add(b);
    }
    // Behind the button: reserve the slot now, fill it once the button's rect is known. Dark only, where light
    // reads as a smudge; a disabled button does not glow.
    let glow = (variant == ButtonVariant::Primary && dark && ui.is_enabled())
        .then(|| ui.painter().add(Shape::Noop));
    // An explicit `Button::fill` would paint in every state, so the fills go through scoped visuals instead.
    let response = ui
        .scope(|ui| {
            let w = &mut ui.visuals_mut().widgets;
            // Only the fill answers hover; the theme's hover outline would turn a flat button into a boxed one.
            w.hovered.bg_stroke = Stroke::NONE;
            w.active.bg_stroke = Stroke::NONE;
            if let Some([rest, hover, press]) = state_fills(variant, dark) {
                w.inactive.bg_stroke = Stroke::NONE;
                // A disabled button is drawn with the noninteractive visuals; it keeps its resting fill and egui fades it.
                for (state, fill) in [
                    (&mut w.noninteractive, rest),
                    (&mut w.inactive, rest),
                    (&mut w.hovered, hover),
                    (&mut w.active, press),
                ] {
                    state.weak_bg_fill = fill;
                    state.bg_fill = fill;
                }
                w.noninteractive.bg_stroke = Stroke::NONE;
            }
            ui.add(b)
        })
        .inner;
    if let Some(slot) = glow {
        let shape = fui::glow_shape(response.rect, tokens(dark).fill, dark);
        ui.painter().set(slot, shape);
    }
    response
}

/// Resting, hovered and pressed fills of the variants that paint their own; `None` leaves the theme's.
/// Hover shades are mixed from tokens, so they follow the theme with no literal colour.
fn state_fills(variant: ButtonVariant, dark: bool) -> Option<[Color32; 3]> {
    let p = tokens(dark);
    match variant {
        ButtonVariant::Primary => Some([
            p.fill,
            p.fill.lerp_to_gamma(p.text_high, 0.2),
            p.fill.lerp_to_gamma(p.text_high, 0.35),
        ]),
        ButtonVariant::Danger => Some([
            p.danger,
            p.danger.lerp_to_gamma(p.text_high, 0.2),
            p.danger.lerp_to_gamma(p.text_high, 0.35),
        ]),
        ButtonVariant::Default | ButtonVariant::Ghost | ButtonVariant::QuietDanger => None,
    }
}

/// A button that stays highlighted while `pressed` (filter chips).
pub fn toggle(ui: &mut Ui, label: &str, pressed: bool) -> Response {
    ui.add(egui::Button::new(label).selected(pressed))
}

/// A pick-one-or-many chip: the selected one is the accent (tint, border, accent text), what the person picked.
/// Reports itself as selected for screen readers and tests.
pub fn chip(ui: &mut Ui, label: impl Into<egui::WidgetText>, selected: bool) -> Response {
    let dark = ui.visuals().dark_mode;
    let p = tokens(dark);
    if !selected {
        return ui.add(egui::Button::new(label).selected(false));
    }
    let tint = Color32::from_rgba_unmultiplied(
        p.fill.r(),
        p.fill.g(),
        p.fill.b(),
        if dark { 31 } else { 23 },
    );
    // Text and rich text (a font chip keeps its own face) take the accent text; a prebuilt layout keeps its own.
    let label = match label.into() {
        egui::WidgetText::Text(s) => egui::WidgetText::from(RichText::new(s).color(p.accent_text)),
        egui::WidgetText::RichText(r) => egui::WidgetText::from((*r).clone().color(p.accent_text)),
        other => other,
    };
    ui.add(
        egui::Button::new(label)
            .selected(true)
            .fill(tint)
            .stroke(egui::Stroke::new(1.0, p.fill)),
    )
}
