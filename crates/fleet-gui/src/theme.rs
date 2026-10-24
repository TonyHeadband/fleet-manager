//! Colours and visuals. The tokens equal the CSS variables in `web/src/styles.css` (a test compares them),
//! so the two front ends read alike.
use eframe::egui::{self, Color32};
use fleet_api::Tone;

pub struct Palette {
    pub bg: Color32,
    pub surface: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub border: Color32,
    pub danger: Color32,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette { bg: rgb(0x12161d), surface: rgb(0x1b212b), text: rgb(0xe6e9ef), muted: rgb(0x9aa5b8), border: rgb(0x2e3644), danger: rgb(0xff7b72) }
    } else {
        Palette { bg: rgb(0xf6f7f9), surface: rgb(0xffffff), text: rgb(0x1c2330), muted: rgb(0x5d6879), border: rgb(0xd9dee6), danger: rgb(0xb42318) }
    }
}

pub fn tone_color(tone: Tone, dark: bool) -> Color32 {
    match (tone, dark) {
        (Tone::Neutral, _) => rgb(0x8a94a6),
        (Tone::Active, false) => rgb(0x2f6fed),
        (Tone::Active, true) => rgb(0x6b9bff),
        (Tone::Attention, false) => rgb(0xd97706),
        (Tone::Attention, true) => rgb(0xf5a524),
        (Tone::Done, false) => rgb(0x15803d),
        (Tone::Done, true) => rgb(0x4ade80),
        (Tone::Muted, false) => rgb(0x7c5cc4),
        (Tone::Muted, true) => rgb(0xb39ddb),
    }
}

fn visuals(dark: bool) -> egui::Visuals {
    let p = palette(dark);
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = p.bg;
    v.window_fill = p.surface;
    v.extreme_bg_color = p.surface;
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, p.border);
    v
}

/// Installs the palette for both themes; egui then follows the system's light/dark choice.
pub fn apply(ctx: &egui::Context) {
    // Agent output can hold any character; a missing glyph must draw a box, never stop the window.
    ctx.set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
    ctx.set_visuals_of(egui::Theme::Dark, visuals(true));
    ctx.set_visuals_of(egui::Theme::Light, visuals(false));
}
