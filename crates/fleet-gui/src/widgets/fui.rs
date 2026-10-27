//! The FUI pieces: thin lines, mono labels and fake glow, each in colours the caller takes from the theme.
use crate::theme::{self, size};
use eframe::egui::{
    self, accesskit::Role, epaint::Mesh, text::LayoutJob, Color32, Painter, Pos2, Rangef, Rect,
    Response, Shape, Stroke, TextFormat, Ui, Vec2, WidgetInfo,
};
use std::f32::consts::{FRAC_PI_2, PI, TAU};

/// An uppercase, letter-spaced mono label: column headers, section labels, persona tags. Assistive tech and tests
/// get the text as written, not shouted.
pub fn mono_label(ui: &mut Ui, text: &str, color: Color32) -> Response {
    mono_label_as(ui, text, text, color)
}

/// `mono_label` that paints `shown` but reports `spoken`: a count painted "02" is announced "2".
pub fn mono_label_as(ui: &mut Ui, shown: &str, spoken: &str, color: Color32) -> Response {
    let response = ui.label(mono_job(shown, color));
    response.widget_info(|| WidgetInfo::labeled(Role::Label, true, spoken));
    response
}

/// `mono_label` cut with "…" when it does not fit, without egui's own tooltip: that one shows the uppercase text,
/// and the caller adds its own with the original case.
pub fn mono_label_truncated(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let response = ui.add(
        egui::Label::new(mono_job(text, color))
            .truncate()
            .show_tooltip_when_elided(false),
    );
    response.widget_info(|| WidgetInfo::labeled(Role::Label, true, text));
    response
}

fn mono_job(shown: &str, color: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.append(
        &shown.to_uppercase(),
        0.0,
        TextFormat {
            font_id: theme::mono(size::META),
            extra_letter_spacing: size::META * 0.13,
            color,
            ..Default::default()
        },
    );
    job
}

/// What a combo box whose selected text is `mono_text` reports: egui fills the value from that text, uppercase, so
/// the value is restated as written.
pub fn combo_info(enabled: bool, name: &str, value: String) -> WidgetInfo {
    let mut info = WidgetInfo::labeled(Role::ComboBox, enabled, name);
    info.current_text_value = Some(value);
    info
}

/// The text of `mono_label` for places that take rich text, such as a combo box's selected value.
pub fn mono_text(text: &str, color: Color32) -> egui::RichText {
    egui::RichText::new(text.to_uppercase())
        .font(theme::mono(size::META))
        .extra_letter_spacing(size::META * 0.13)
        .color(color)
}

/// A 1 px line along the bottom of `rect` with a short tick every 10 px.
pub fn tick_rule(painter: &Painter, rect: Rect, color: Color32) {
    let stroke = Stroke::new(1.0, color);
    let y = rect.bottom();
    painter.line_segment(
        [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
        stroke,
    );
    let mut x = rect.left();
    while x <= rect.right() {
        painter.line_segment([egui::pos2(x, y - 3.0), egui::pos2(x, y)], stroke);
        x += 10.0;
    }
}

/// Ticks every 10 px across `x`, ending one pixel above `bottom` so a border drawn on that row stays clean.
pub fn ticks(painter: &Painter, x: Rangef, bottom: f32, color: Color32) {
    painter.add(ticks_shape(x, bottom, color));
}

/// `ticks` as one shape, for a caller that reserves its slot behind other widgets with `Painter::add(Shape::Noop)`.
pub fn ticks_shape(x: Rangef, bottom: f32, color: Color32) -> Shape {
    let stroke = Stroke::new(1.0, color);
    let mut shapes = Vec::new();
    let mut at = x.min;
    while at <= x.max {
        shapes.push(Shape::line_segment(
            [egui::pos2(at, bottom - 4.0), egui::pos2(at, bottom - 1.0)],
            stroke,
        ));
        at += 10.0;
    }
    Shape::Vec(shapes)
}

/// What marks the selected item: a wash and a hairline border that are strong at the right and fade out leftward,
/// where the status edge sits. Drawn inside `rect` and clear of its `rounding` corners.
pub fn selection_fade(painter: &Painter, rect: Rect, accent: Color32, dark: bool, rounding: f32) {
    let tint = |a: f32| {
        Color32::from_rgba_unmultiplied(
            accent.r(),
            accent.g(),
            accent.b(),
            (a * 255.0).round() as u8,
        )
    };
    let (top, bottom) = (rect.top(), rect.bottom());
    let (left, right) = (rect.left(), rect.right());
    let mid = right - rect.width() * 0.45;

    // Fill: stops at the right (1.0), 0.45 in, and the left edge. The right column stops short of the corners.
    let [a_right, a_mid] = if dark { [0.26, 0.08] } else { [0.18, 0.05] };
    let mut fill = Mesh::default();
    let columns = [
        (left, top, bottom, tint(0.0)),
        (mid, top, bottom, tint(a_mid)),
        (right, top + rounding, bottom - rounding, tint(a_right)),
    ];
    for (x, y0, y1, c) in columns {
        fill.colored_vertex(egui::pos2(x, y0), c);
        fill.colored_vertex(egui::pos2(x, y1), c);
    }
    fill.add_triangle(0, 1, 2);
    fill.add_triangle(1, 3, 2);
    fill.add_triangle(2, 3, 4);
    fill.add_triangle(3, 5, 4);
    painter.add(Shape::mesh(fill));

    // Border: 1 px strips, hand-built because a stroke cannot fade along its length.
    let edge_end = right - rounding;
    let stops = [
        (left, 0.0),
        ((left + edge_end) / 2.0, 0.55),
        (edge_end, 1.0),
    ];
    for (y0, y1) in [(top, top + 1.0), (bottom - 1.0, bottom)] {
        let mut strip = Mesh::default();
        for (i, (x, a)) in stops.into_iter().enumerate() {
            strip.colored_vertex(egui::pos2(x, y0), tint(a));
            strip.colored_vertex(egui::pos2(x, y1), tint(a));
            if i > 0 {
                let n = 2 * i as u32;
                strip.add_triangle(n - 2, n - 1, n);
                strip.add_triangle(n - 1, n + 1, n);
            }
        }
        painter.add(Shape::mesh(strip));
    }
    let mut side = Mesh::default();
    for (x, y) in [
        (right - 1.0, top + rounding),
        (right, top + rounding),
        (right - 1.0, bottom - rounding),
        (right, bottom - rounding),
    ] {
        side.colored_vertex(egui::pos2(x, y), accent);
    }
    side.add_triangle(0, 1, 2);
    side.add_triangle(1, 3, 2);
    painter.add(Shape::mesh(side));

    // Dark only: light spilling past the right edge, never over the fill. Inset from the corners like a shadow with a
    // negative spread; the widest layer is the faintest.
    if dark {
        let inset = rounding.max(4.0);
        for (layer, reach) in theme::glow(accent, true).into_iter().zip([6.0, 4.0, 2.0]) {
            let strip = Rect::from_min_max(
                egui::pos2(right, top + inset),
                egui::pos2(right + reach, bottom - inset),
            );
            painter.rect_filled(
                strip,
                egui::CornerRadius {
                    nw: 0,
                    sw: 0,
                    ne: reach as u8,
                    se: reach as u8,
                },
                layer,
            );
        }
    }
}

/// Four L-shaped corners just inside `rect`: what marks the selected item.
pub fn corner_brackets(painter: &Painter, rect: Rect, color: Color32) {
    const ARM: f32 = 7.0;
    const INSET: f32 = 3.0;
    let r = rect.shrink(INSET);
    let stroke = Stroke::new(1.0, color);
    for (corner, dx, dy) in [
        (r.left_top(), 1.0, 1.0),
        (r.right_top(), -1.0, 1.0),
        (r.left_bottom(), 1.0, -1.0),
        (r.right_bottom(), -1.0, -1.0),
    ] {
        painter.add(Shape::line(
            vec![
                corner + Vec2::new(0.0, ARM * dy),
                corner,
                corner + Vec2::new(ARM * dx, 0.0),
            ],
            stroke,
        ));
    }
}

/// A glow behind `rect`: `theme::glow`'s layers as rounded rects, widest and faintest first. Only for live things.
pub fn glow_rect(painter: &Painter, rect: Rect, color: Color32, dark: bool) {
    painter.add(glow_shape(rect, color, dark));
}

/// The layers of `glow_rect` as one shape, for a caller that reserves its slot behind a widget with
/// `Painter::add(Shape::Noop)` and fills it once the widget's rect is known.
pub fn glow_shape(rect: Rect, color: Color32, dark: bool) -> Shape {
    Shape::Vec(
        theme::glow(color, dark)
            .into_iter()
            .zip([6.0, 4.0, 2.0])
            .map(|(layer, spread)| {
                Shape::rect_filled(
                    rect.expand(spread),
                    theme::radius::CARD as f32 + spread,
                    layer,
                )
            })
            .collect(),
    )
}

/// The icon drawn in one colour: the craft (nose, C blade, tail) and its radar, the radar turned by `angle`
/// radians. `size` is the side of the icon's 64-unit square, centred on `center`.
pub fn radar_mark(painter: &Painter, center: Pos2, size: f32, color: Color32, angle: f32) {
    let k = size / 64.0;
    // Icon units, (0,0) top left of the 64 square, to screen.
    let at = |x: f32, y: f32| center + Vec2::new(x - 32.0, y - 32.0) * k;
    let hub = (32.0, 34.0);
    // A point `r` units from the radar's hub in direction `a` (0 = up, clockwise), turned by `angle`.
    let polar = |r: f32, a: f32| {
        let a = a + angle - FRAC_PI_2;
        at(hub.0 + r * a.cos(), hub.1 + r * a.sin())
    };
    let thin = Stroke::new((0.6 * k).max(1.0), color);

    // Radar, back to front: the sweep wedge, its leading line, the guide circle, the dotted ring, the ticks.
    let wedge = 35f32.to_radians();
    let mut fan = vec![polar(0.0, 0.0)];
    fan.extend((0..=12).map(|i| polar(23.0, wedge * i as f32 / 12.0)));
    painter.add(Shape::convex_polygon(
        fan,
        color.gamma_multiply(0.35),
        Stroke::NONE,
    ));
    painter.line_segment([polar(0.0, 0.0), polar(23.0, 0.0)], thin);
    painter.circle_stroke(at(hub.0, hub.1), 23.0 * k, thin);
    let dots = (TAU * 26.5 / 3.0) as usize;
    for i in 0..dots {
        painter.circle_filled(
            polar(26.5, TAU * i as f32 / dots as f32),
            (0.35 * k).max(0.5),
            color,
        );
    }
    for i in 0..4 {
        let a = FRAC_PI_2 * i as f32;
        painter.line_segment([polar(24.5, a), polar(26.5, a)], thin);
    }

    // The craft stays still.
    painter.add(Shape::convex_polygon(
        vec![at(32.0, 11.0), at(35.5, 30.0), at(28.5, 30.0)],
        color,
        Stroke::NONE,
    ));
    painter.add(Shape::convex_polygon(
        vec![at(29.5, 41.5), at(34.5, 41.5), at(32.0, 46.5)],
        color,
        Stroke::NONE,
    ));
    // The C blade: the crescent between two upper half-ellipses on the same chord, (17,51) to (47,51). It is not
    // convex, so it goes in as a strip of triangles.
    const STEPS: u32 = 24;
    let mut blade = Mesh::default();
    for i in 0..=STEPS {
        let a = PI * i as f32 / STEPS as f32;
        let x = 32.0 - 15.0 * a.cos();
        blade.colored_vertex(at(x, 51.0 - 18.0 * a.sin()), color);
        blade.colored_vertex(at(x, 51.0 - 15.5 * a.sin()), color);
        if i > 0 {
            let n = 2 * i;
            blade.add_triangle(n - 2, n - 1, n);
            blade.add_triangle(n - 1, n + 1, n);
        }
    }
    painter.add(Shape::mesh(blade));
}
