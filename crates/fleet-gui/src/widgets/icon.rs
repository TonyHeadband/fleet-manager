use crate::theme::{radius, tokens, tone_color, tone_tint};
use eframe::egui::{
    self, pos2, vec2, Color32, FontId, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Ui,
    WidgetInfo,
};
use fleet_api::Tone;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Close,
    Info,
    Pause,
    Resume,
    Stop,
    /// ◎: an orchestrator, the lead of a project.
    Orchestrator,
    /// </>: a specialist that writes code.
    Code,
    Eye,
    Flask,
    /// ◇: a specialist with no icon of its own.
    Specialist,
    /// ⇡: a pull request is open.
    PullRequest,
    /// Brackets at the corners: enlarge the panel.
    Expand,
    /// Brackets pulled to the centre: give the board back.
    Collapse,
}

const SIZE: f32 = 26.0;
const GLYPH: f32 = 4.5;
/// A dense row's icon button (a card's actions): small enough that two fit half a narrow card.
const SMALL: f32 = 20.0;
const SMALL_GLYPH: f32 = 4.5;

/// `icon` in `ink`, centred on `c`, reaching `half` from the centre each way.
fn glyph(painter: &Painter, icon: Icon, c: Pos2, half: f32, ink: Color32) {
    let stroke = Stroke::new(1.5, ink);
    match icon {
        Icon::Close => {
            painter.line_segment([c + vec2(-half, -half), c + vec2(half, half)], stroke);
            painter.line_segment([c + vec2(-half, half), c + vec2(half, -half)], stroke);
        }
        Icon::Info => {
            painter.circle_stroke(c, half + 1.0, Stroke::new(1.2, ink));
            painter.circle_filled(c + vec2(0.0, -half * 0.5), 0.9, ink);
            painter.line_segment(
                [c + vec2(0.0, -half * 0.1), c + vec2(0.0, half * 0.6)],
                Stroke::new(1.3, ink),
            );
        }
        Icon::Pause => {
            for x in [-half * 0.5, half * 0.5] {
                let bar = Rect::from_center_size(c + vec2(x, 0.0), vec2(half * 0.55, half * 1.8));
                painter.rect_filled(bar, 0.5, ink);
            }
        }
        Icon::Resume => {
            // Nudged right: a triangle's visual centre sits left of its box's.
            let c = c + vec2(half * 0.15, 0.0);
            painter.add(Shape::convex_polygon(
                vec![
                    pos2(c.x - half * 0.75, c.y - half),
                    pos2(c.x + half, c.y),
                    pos2(c.x - half * 0.75, c.y + half),
                ],
                ink,
                Stroke::NONE,
            ));
        }
        Icon::Stop => {
            painter.rect_filled(Rect::from_center_size(c, vec2(half, half) * 1.6), 1.0, ink);
        }
        Icon::Orchestrator => {
            painter.circle_stroke(c, half + 0.5, Stroke::new(1.2, ink));
            painter.circle_filled(c, half * 0.45, ink);
        }
        Icon::Code => {
            let s = Stroke::new(1.3, ink);
            for side in [-1.0, 1.0] {
                let p = |x: f32, y: f32| c + vec2(side * x, y);
                painter.line_segment([p(half * 0.35, -half * 0.75), p(half, 0.0)], s);
                painter.line_segment([p(half, 0.0), p(half * 0.35, half * 0.75)], s);
            }
        }
        Icon::Eye => {
            let outline: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let t = std::f32::consts::TAU * i as f32 / 24.0;
                    c + vec2(half * 1.1 * t.cos(), half * 0.6 * t.sin())
                })
                .collect();
            painter.add(Shape::line(outline, Stroke::new(1.2, ink)));
            painter.circle_filled(c, half * 0.35, ink);
        }
        Icon::Flask => {
            let p = |x: f32, y: f32| c + vec2(x * half, y * half);
            painter.add(Shape::closed_line(
                vec![
                    p(-0.3, -1.0),
                    p(0.3, -1.0),
                    p(0.3, -0.3),
                    p(1.0, 1.0),
                    p(-1.0, 1.0),
                    p(-0.3, -0.3),
                ],
                Stroke::new(1.2, ink),
            ));
        }
        Icon::Specialist => {
            painter.add(Shape::closed_line(
                vec![
                    c + vec2(0.0, -half),
                    c + vec2(half, 0.0),
                    c + vec2(0.0, half),
                    c + vec2(-half, 0.0),
                ],
                Stroke::new(1.2, ink),
            ));
        }
        Icon::Expand | Icon::Collapse => {
            // Four corner brackets: out at the edges to expand, pulled in towards the centre to collapse.
            let s = Stroke::new(1.3, ink);
            let (outer, inner) = (half, half * 0.3);
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let tip = if icon == Icon::Expand { outer } else { inner };
                let arm = half * 0.7;
                let corner = c + vec2(sx * tip, sy * tip);
                let dir = if icon == Icon::Expand { -1.0 } else { 1.0 };
                painter.line_segment([corner, corner + vec2(dir * sx * arm, 0.0)], s);
                painter.line_segment([corner, corner + vec2(0.0, dir * sy * arm)], s);
            }
        }
        Icon::PullRequest => {
            let s = Stroke::new(1.3, ink);
            painter.line_segment([c + vec2(0.0, half), c + vec2(0.0, -half)], s);
            painter.line_segment(
                [c + vec2(-half * 0.6, -half * 0.4), c + vec2(0.0, -half)],
                s,
            );
            painter.line_segment([c + vec2(half * 0.6, -half * 0.4), c + vec2(0.0, -half)], s);
        }
    }
}

/// `icon` as a 14 px mark in a row of text, not a button. `label` is the accessible name and the tooltip.
pub fn mark(ui: &mut Ui, icon: Icon, label: &str) -> Response {
    let ink = tokens(ui.visuals().dark_mode).text_low;
    let (rect, response) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
    glyph(ui.painter(), icon, rect.center(), GLYPH, ink);
    response.widget_info(|| WidgetInfo::labeled(egui::accesskit::Role::Image, true, label));
    response.on_hover_text(label)
}

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
    let painter = ui.painter();
    painter.rect(
        rect,
        frame.corner_radius,
        frame.bg_fill,
        Stroke::new(1.0, p.border),
        egui::StrokeKind::Inside,
    );

    glyph(painter, icon, rect.center(), GLYPH, ink);

    response
        .widget_info(|| WidgetInfo::labeled(egui::accesskit::Role::Button, ui.is_enabled(), label));
    response.on_hover_text(label)
}

/// A frameless `SMALL` icon button for dense rows. `danger` paints it red on a red tint: an armed destructive
/// action waiting for its second click. `label` is the accessible name and the tooltip.
pub fn small_icon_button(ui: &mut Ui, icon: Icon, label: &str, danger: bool) -> Response {
    let dark = ui.visuals().dark_mode;
    let p = tokens(dark);
    let (rect, response) = ui.allocate_exact_size(vec2(SMALL, SMALL), Sense::click());
    let (fill, ink) = if danger {
        (tone_tint(Tone::Danger, dark), p.danger)
    } else if response.hovered() {
        (p.raised, p.text_high)
    } else {
        (Color32::TRANSPARENT, p.text_low)
    };
    let painter = ui.painter();
    painter.rect_filled(rect, radius::CONTROL, fill);
    glyph(painter, icon, rect.center(), SMALL_GLYPH, ink);
    response
        .widget_info(|| WidgetInfo::labeled(egui::accesskit::Role::Button, ui.is_enabled(), label));
    response.on_hover_text(label)
}

/// ⚠ painted (no bundled font carries it): an amber triangle with the `surface` exclamation mark in it.
pub fn warn_mark(ui: &mut Ui) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
    let c = rect.center();
    let p = tokens(ui.visuals().dark_mode);
    let amber = tone_color(Tone::Attention, ui.visuals().dark_mode);
    ui.painter().add(Shape::convex_polygon(
        vec![c + vec2(0.0, -6.0), c + vec2(6.5, 5.5), c + vec2(-6.5, 5.5)],
        amber,
        Stroke::NONE,
    ));
    ui.painter().line_segment(
        [pos2(c.x, c.y - 2.0), pos2(c.x, c.y + 1.5)],
        Stroke::new(1.5, p.surface),
    );
    ui.painter()
        .circle_filled(pos2(c.x, c.y + 3.5), 0.9, p.surface);
    response
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
