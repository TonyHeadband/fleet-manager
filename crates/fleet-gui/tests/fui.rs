use eframe::egui;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use fleet_gui::theme::{self, tokens};
use fleet_gui::widgets::fui;
use std::cell::RefCell;
use std::rc::Rc;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn pulse_is_zero_at_ends_and_one_mid() {
    let p = 1.8;
    assert!(close(theme::pulse_at(0.0, p), 0.0));
    assert!(close(theme::pulse_at(p / 2.0, p), 1.0));
    assert!(close(theme::pulse_at(p, p), 0.0));
    // Eased: slow at the ends, so a quarter of the way in it is still below the linear half.
    assert!(theme::pulse_at(p / 8.0, p) < 0.25);
}

#[test]
fn visor_ping_pongs() {
    let p = 1.3;
    assert!(close(theme::visor_at(0.0, p), 0.0));
    assert!(close(theme::visor_at(p, p), 1.0));
    assert!(close(theme::visor_at(2.0 * p, p), 0.0));
    // Going down, then back up over the same ground.
    assert!(close(
        theme::visor_at(0.3 * p, p),
        theme::visor_at(1.7 * p, p)
    ));
    for i in 0..=40 {
        let v = theme::visor_at(i as f32 * 0.1, p);
        assert!((0.0..=1.0).contains(&v), "visor {v} at step {i}");
    }
}

#[test]
fn clocks_are_none_when_animations_off() {
    let seen = Rc::new(RefCell::new(vec![Some(0.0f32); 3]));
    let s = seen.clone();
    let mut h = Harness::new_ui(move |ui| {
        theme::set_animations(ui.ctx(), false);
        *s.borrow_mut() = vec![
            theme::pulse(ui, 1.8),
            theme::visor(ui, 1.3),
            theme::spin(ui, 40.0),
        ];
    });
    h.input_mut()
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .focused = Some(true);
    h.step();
    assert_eq!(*seen.borrow(), vec![None, None, None]);
    assert!(!h.ctx.has_requested_repaint());
}

#[test]
fn clocks_run_while_focused() {
    let seen = Rc::new(RefCell::new(vec![None; 3]));
    let s = seen.clone();
    let mut h = Harness::new_ui(move |ui| {
        *s.borrow_mut() = vec![
            theme::pulse(ui, 1.8),
            theme::visor(ui, 1.3),
            theme::spin(ui, 40.0),
        ];
    });
    h.input_mut()
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .focused = Some(true);
    h.input_mut().time = Some(10.0);
    // `step`, not `run`: a running clock schedules a repaint, which would make `run` loop.
    h.step();
    for v in seen.borrow().iter() {
        let v = v.expect("runs while focused");
        assert!((0.0..=1.0).contains(&v), "{v}");
    }
    assert!(close(seen.borrow()[2].unwrap(), 0.25), "spin is linear");
}

#[test]
fn glow_layers_fade_outward() {
    for dark in [true, false] {
        let c = tokens(dark).fill;
        let layers = theme::glow(c, dark);
        let alphas = layers.map(|l| l.a());
        assert!(
            alphas[0] < alphas[1] && alphas[1] < alphas[2],
            "dark={dark}: {alphas:?} must grow inward"
        );
        for l in layers {
            let [r, g, b, a] = l.to_srgba_unmultiplied();
            // Premultiplied storage keeps about 255/alpha steps of precision per channel at low alpha.
            let slack = (255 / a.max(1)) + 1;
            for (got, want) in [(r, c.r()), (g, c.g()), (b, c.b())] {
                assert!(
                    got.abs_diff(want) <= slack,
                    "dark={dark}: {l:?} is not {c:?}"
                );
            }
        }
    }
}

fn shapes_of(paint: impl Fn(&mut egui::Ui) + 'static) -> Vec<egui::epaint::ClippedShape> {
    let mut h = Harness::new_ui(move |ui| paint(ui));
    h.run();
    h.output().shapes.clone()
}

fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
    fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().to_owned()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    shapes.iter().for_each(|s| walk(&s.shape, &mut out));
    out
}

#[test]
fn helpers_paint_in_both_themes() {
    for dark in [true, false] {
        let t = tokens(dark);
        let area = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(200.0, 80.0));
        type Paint = Box<dyn Fn(&egui::Painter)>;
        let helpers: [(&str, Paint); 4] = [
            (
                "tick_rule",
                Box::new(move |p| fui::tick_rule(p, area, t.dashed)),
            ),
            (
                "corner_brackets",
                Box::new(move |p| fui::corner_brackets(p, area, t.fill)),
            ),
            (
                "glow_rect",
                Box::new(move |p| fui::glow_rect(p, area, t.fill, dark)),
            ),
            (
                "radar_mark",
                Box::new(move |p| fui::radar_mark(p, egui::pos2(120.0, 120.0), 240.0, t.fill, 0.7)),
            ),
        ];
        for (name, paint) in helpers {
            let shapes = shapes_of(move |ui| paint(ui.painter()));
            assert!(!shapes.is_empty(), "dark={dark}: {name} painted nothing");
        }
        let shapes = shapes_of(move |ui| {
            fui::mono_label(ui, "Needs you", t.text_low);
        });
        assert_eq!(texts(&shapes), vec!["NEEDS YOU".to_owned()], "dark={dark}");
    }
}

#[test]
fn radar_mark_rotates_only_the_radar() {
    let paint = |angle: f32| {
        shapes_of(move |ui| {
            fui::radar_mark(
                ui.painter(),
                egui::pos2(120.0, 120.0),
                240.0,
                egui::Color32::WHITE,
                angle,
            )
        })
    };
    let (a, b) = (paint(0.0), paint(1.0));
    assert_eq!(a.len(), b.len(), "same parts at any angle");
    let differ = a
        .iter()
        .zip(&b)
        .filter(|(x, y)| format!("{:?}", x.shape) != format!("{:?}", y.shape))
        .count();
    assert!(differ > 0, "the radar must move");
    assert!(differ < a.len(), "the craft must stay still");
}

fn leaves(shapes: &[egui::epaint::ClippedShape]) -> Vec<egui::Shape> {
    fn walk(shape: &egui::Shape, out: &mut Vec<egui::Shape>) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            other => out.push(other.clone()),
        }
    }
    let mut out = Vec::new();
    shapes.iter().for_each(|s| walk(&s.shape, &mut out));
    out
}

#[test]
fn mono_label_keeps_its_label_for_assistive_tech() {
    let t = tokens(true);
    let mut h = Harness::new_ui(move |ui| {
        fui::mono_label(ui, "Needs you", t.text_low);
    });
    h.run();
    h.get_by_label("Needs you");
    assert_eq!(texts(&h.output().shapes), vec!["NEEDS YOU".to_owned()]);
}

#[test]
fn ticks_stay_off_the_bottom_pixel_row() {
    let bottom = 50.0;
    let shapes = shapes_of(move |ui| {
        fui::ticks(
            ui.painter(),
            egui::Rangef::new(10.0, 60.0),
            bottom,
            egui::Color32::WHITE,
        )
    });
    let segs: Vec<[egui::Pos2; 2]> = leaves(&shapes)
        .into_iter()
        .filter_map(|s| match s {
            egui::Shape::LineSegment { points, .. } => Some(points),
            _ => None,
        })
        .collect();
    assert_eq!(segs.len(), 6, "10, 20 .. 60");
    for (i, [a, b]) in segs.iter().enumerate() {
        assert!(close(a.x, 10.0 + 10.0 * i as f32), "spacing: {a:?}");
        assert!(close(a.y, bottom - 4.0) && close(a.x, b.x), "{a:?} {b:?}");
        assert!(b.y <= bottom - 1.0, "tick reaches the bottom row: {b:?}");
    }
}

#[test]
fn selection_fade_is_strong_right_and_gone_left() {
    let rect = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(200.0, 80.0));
    let accent = egui::Color32::from_rgb(40, 200, 120);
    for dark in [true, false] {
        let shapes =
            shapes_of(move |ui| fui::selection_fade(ui.painter(), rect, accent, dark, 6.0));
        let all = leaves(&shapes);
        let verts: Vec<egui::epaint::Vertex> = all
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Mesh(m) => Some(m.vertices.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        assert!(!verts.is_empty(), "dark={dark}");
        for v in &verts {
            assert!(
                rect.expand(1e-3).contains(v.pos),
                "dark={dark}: {v:?} outside"
            );
        }
        let max_x = verts.iter().map(|v| v.pos.x).fold(f32::MIN, f32::max);
        let min_x = verts.iter().map(|v| v.pos.x).fold(f32::MAX, f32::min);
        assert!(close(min_x, rect.left()) && close(max_x, rect.right()));
        let right: Vec<_> = verts.iter().filter(|v| close(v.pos.x, max_x)).collect();
        assert!(
            right.iter().any(|v| v.color.a() == 255),
            "dark={dark}: border"
        );
        let want = if dark { 26.0 } else { 18.0 } / 100.0 * 255.0;
        assert!(
            right
                .iter()
                .any(|v| (v.color.a() as f32 - want).abs() <= 2.0),
            "dark={dark}: fill's right stop"
        );
        assert!(
            right.iter().all(|v| {
                // Premultiplied storage: low alpha leaves ~255/alpha steps of precision per channel.
                let [r, ..] = v.color.to_srgba_unmultiplied();
                r.abs_diff(accent.r()) as u32 <= 255 / v.color.a().max(1) as u32 + 1
            }),
            "dark={dark}: accent hue"
        );
        for v in verts.iter().filter(|v| close(v.pos.x, min_x)) {
            assert_eq!(v.color.a(), 0, "dark={dark}: left edge {v:?}");
        }
        // The harness paints a background of its own, so count glow rects against an empty baseline.
        let non_mesh = |s: &[egui::Shape]| {
            s.iter()
                .filter(|s| !matches!(s, egui::Shape::Mesh(_)))
                .count()
        };
        let baseline = non_mesh(&leaves(&shapes_of(|_| {})));
        let glow: Vec<_> = all
            .iter()
            .filter(|s| !matches!(s, egui::Shape::Mesh(_)))
            .skip(baseline)
            .collect();
        if dark {
            let rects: Vec<&egui::epaint::RectShape> = glow
                .iter()
                .map(|s| match s {
                    egui::Shape::Rect(r) => r,
                    other => panic!("glow is rects, got {other:?}"),
                })
                .collect();
            assert_eq!(rects.len(), 3, "dark: three glow layers");
            for r in &rects {
                assert!(
                    r.rect.min.x >= rect.right() - 1e-3,
                    "glow over the rect: {r:?}"
                );
                assert!(r.rect.max.y <= rect.bottom() && r.rect.min.y >= rect.top());
            }
            let widths: Vec<f32> = rects.iter().map(|r| r.rect.width()).collect();
            assert!(widths[0] > widths[1] && widths[1] > widths[2], "{widths:?}");
            let alphas: Vec<u8> = rects.iter().map(|r| r.fill.a()).collect();
            assert!(alphas[0] < alphas[1] && alphas[1] < alphas[2], "{alphas:?}");
        } else {
            assert!(glow.is_empty(), "light has no glow");
        }
    }
}

#[test]
fn visor_band_is_lighter_active() {
    use fleet_api::Tone;
    for dark in [true, false] {
        let active = theme::tone_color(Tone::Active, dark);
        let band = theme::visor_band(dark);
        let white = egui::Color32::WHITE;
        for (b, a, w) in [
            (band.r(), active.r(), white.r()),
            (band.g(), active.g(), white.g()),
            (band.b(), active.b(), white.b()),
        ] {
            assert!(a <= b && b <= w, "dark={dark}: {a} <= {b} <= {w}");
        }
        assert_ne!(band, active);
        assert_ne!(band, white);
    }
}
