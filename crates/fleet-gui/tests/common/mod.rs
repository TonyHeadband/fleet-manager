// Each test crate uses a subset of the helpers.
#![allow(dead_code)]
pub mod log;

/// Whether a painted text is `text` in Monospace.
pub fn paints_mono<S>(h: &egui_kittest::Harness<'_, S>, text: &str) -> bool {
    fn walk(s: &eframe::egui::Shape, text: &str) -> bool {
        match s {
            eframe::egui::Shape::Text(t) => {
                t.galley.job.text == text
                    && t.galley.job.sections[0].format.font_id.family
                        == eframe::egui::FontFamily::Monospace
            }
            eframe::egui::Shape::Vec(v) => v.iter().any(|s| walk(s, text)),
            _ => false,
        }
    }
    h.output().shapes.iter().any(|s| walk(&s.shape, text))
}
