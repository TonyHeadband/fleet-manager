use crate::theme::tokens;
use eframe::egui::{self, vec2, FontId, Response, Sense, Stroke, Ui, WidgetInfo};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Close,
}

const SIZE: f32 = 26.0;
const GLYPH: f32 = 4.5;

/// A square button with a painted glyph. Glyphs are painted, not typed: no font has to carry them.
/// `label` is the accessible name and the tooltip.
pub fn icon_button(ui: &mut Ui, icon: Icon, label: &str) -> Response {
    let p = tokens(ui.visuals().dark_mode);
    let (rect, response) = ui.allocate_exact_size(vec2(SIZE, SIZE), Sense::click());
    // The frame follows the theme's widget states, so it hovers like a Default button.
    let frame = ui.style().interact(&response);
    let ink = if response.hovered() {
        p.text_high
    } else {
        p.text_low
    };
    let stroke = Stroke::new(1.5, ink);
    let painter = ui.painter();
    painter.rect(
        rect,
        frame.corner_radius,
        frame.bg_fill,
        Stroke::new(1.0, p.border),
        egui::StrokeKind::Inside,
    );

    let c = rect.center();
    match icon {
        Icon::Close => {
            painter.line_segment([c + vec2(-GLYPH, -GLYPH), c + vec2(GLYPH, GLYPH)], stroke);
            painter.line_segment([c + vec2(-GLYPH, GLYPH), c + vec2(GLYPH, -GLYPH)], stroke);
        }
    }

    response
        .widget_info(|| WidgetInfo::labeled(egui::accesskit::Role::Button, ui.is_enabled(), label));
    response.on_hover_text(label)
}

/// A painted filled circle (live indicator, column dots); allocates `diameter` × `diameter`.
pub fn dot(ui: &mut Ui, color: egui::Color32, diameter: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(diameter, diameter), Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), diameter / 2.0, color);
    response
}

/// A dot that leads a line of `font` text: it takes the text's row height and sits in its middle. A bare `dot`
/// is shorter than the text, so a row that is only as tall as its first item would leave it at the top.
pub fn text_dot(ui: &mut Ui, color: egui::Color32, diameter: f32, font: &FontId) -> Response {
    let height = ui.fonts_mut(|f| f.row_height(font)).max(diameter);
    let (rect, response) = ui.allocate_exact_size(vec2(diameter, height), Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), diameter / 2.0, color);
    response
}
