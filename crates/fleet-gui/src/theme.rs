//! Colours, fonts and visuals. Nothing outside this file writes a literal colour: widgets read `tokens`,
//! and egui's stock widgets (TextEdit, ComboBox, Modal, ScrollArea) pick the same values up from `Visuals`.
use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Shadow, Stroke,
    TextStyle, Ui,
};
use fleet_api::Tone;
use fleet_client::config::ThemeChoice;
use std::sync::Arc;

#[derive(Debug, Clone, Copy)]
pub struct Tokens {
    pub bg: Color32,
    pub surface: Color32,
    pub raised: Color32,
    /// Darker than `bg`: the body of a column of things that are no longer running.
    pub sunken: Color32,
    pub border: Color32,
    pub text_high: Color32,
    pub text: Color32,
    pub text_low: Color32,
    /// Primary button background; `on_fill` is its label.
    pub fill: Color32,
    pub on_fill: Color32,
    pub danger: Color32,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub fn tokens(dark: bool) -> Tokens {
    if dark {
        Tokens {
            bg: rgb(0x1c1c1c),
            surface: rgb(0x212121),
            raised: rgb(0x292929),
            sunken: rgb(0x161616),
            border: rgb(0x333333),
            text_high: rgb(0xf5f5f5),
            text: rgb(0xc4c4c4),
            text_low: rgb(0x8f8f8f),
            fill: rgb(0xf5f5f5),
            on_fill: rgb(0x141414),
            danger: rgb(0xe5534b),
        }
    } else {
        Tokens {
            bg: rgb(0xf2f2f2),
            surface: rgb(0xffffff),
            raised: rgb(0xe3e3e3),
            sunken: rgb(0xe8e8e8),
            border: rgb(0xd9d9d9),
            text_high: rgb(0x0d0d0d),
            text: rgb(0x333333),
            text_low: rgb(0x636363),
            fill: rgb(0x141414),
            on_fill: rgb(0xffffff),
            danger: rgb(0xc4302b),
        }
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
        (Tone::Caution, false) => rgb(0xca8a04),
        (Tone::Caution, true) => rgb(0xfacc15),
        // The same reds as the `danger` token.
        (Tone::Danger, false) => rgb(0xc4302b),
        (Tone::Danger, true) => rgb(0xe5534b),
    }
}

/// The tone at low alpha, for badge backgrounds; a touch stronger in dark where a tint reads fainter.
pub fn tone_tint(tone: Tone, dark: bool) -> Color32 {
    let c = tone_color(tone, dark);
    let alpha = if dark { 36 } else { 31 };
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weight {
    Regular,
    Medium,
}

/// egui has no font weights; Medium is its own family holding the Plex Sans Medium file.
pub const MEDIUM_FAMILY: &str = "plex-medium";

const PLEX_SANS: &str = "plex-sans";
const PLEX_SANS_MEDIUM: &str = "plex-sans-medium";
const PLEX_MONO: &str = "plex-mono";

pub mod size {
    pub const HEADING: f32 = 15.0;
    pub const BODY: f32 = 13.0;
    pub const TITLE: f32 = 12.5;
    pub const BUTTON: f32 = 12.0;
    pub const SMALL: f32 = 11.5;
    pub const META: f32 = 10.5;
    pub const CODE: f32 = 11.5;
}

/// The app's name wherever a person reads it. Ids, paths and binaries keep `fleet-manager`.
pub const APP_NAME: &str = "Fleet Manager";

/// The shell's fixed sizes. The drawer's header and footer share the content's heights so their border lines run
/// unbroken across the window.
pub mod layout {
    pub const HEADER: f32 = 40.0;
    pub const FOOTER: f32 = 28.0;
    pub const DRAWER: f32 = 200.0;
}

pub mod radius {
    pub const CONTROL: u8 = 3;
    pub const CARD: u8 = 4;
    pub const DIALOG: u8 = 6;
}

fn medium_family() -> FontFamily {
    FontFamily::Name(MEDIUM_FAMILY.into())
}

/// Plex Sans at `size`. Laying out an unregistered named family panics, and `apply` is not run by widget
/// tests, so Medium falls back to Proportional until the family exists.
pub fn sans(ui: &Ui, size: f32, weight: Weight) -> FontId {
    let family = medium_family();
    let installed = ui
        .ctx()
        .fonts(|f| f.definitions().families.contains_key(&family));
    match weight {
        Weight::Medium if installed => FontId::new(size, family),
        _ => FontId::new(size, FontFamily::Proportional),
    }
}

pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

fn font_definitions() -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    let font = |bytes: &'static [u8]| Arc::new(FontData::from_static(bytes));
    defs.font_data.insert(
        PLEX_SANS.into(),
        font(include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf")),
    );
    defs.font_data.insert(
        PLEX_SANS_MEDIUM.into(),
        font(include_bytes!("../assets/fonts/IBMPlexSans-Medium.ttf")),
    );
    defs.font_data.insert(
        PLEX_MONO.into(),
        font(include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf")),
    );

    // egui's own fonts stay behind Plex: they cover the glyphs (emoji, symbols) Plex lacks.
    let mut medium = vec![PLEX_SANS_MEDIUM.to_owned()];
    medium.extend(defs.families[&FontFamily::Proportional].iter().cloned());
    defs.families.insert(medium_family(), medium);
    defs.families
        .get_mut(&FontFamily::Proportional)
        .expect("egui defines Proportional")
        .insert(0, PLEX_SANS.into());
    defs.families
        .get_mut(&FontFamily::Monospace)
        .expect("egui defines Monospace")
        .insert(0, PLEX_MONO.into());
    defs
}

fn visuals(dark: bool) -> egui::Visuals {
    let t = tokens(dark);
    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.panel_fill = t.bg;
    v.window_fill = t.surface;
    v.extreme_bg_color = t.bg;
    v.faint_bg_color = t.raised;
    v.override_text_color = None;

    let control = CornerRadius::same(radius::CONTROL);
    let w = &mut v.widgets;
    w.noninteractive.bg_stroke = Stroke::new(1.0, t.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, t.text);
    for (state, fill, stroke, text) in [
        (&mut w.inactive, t.surface, t.border, t.text),
        (&mut w.hovered, t.raised, t.text_low, t.text_high),
        (&mut w.active, t.raised, t.text_low, t.text_high),
        (&mut w.open, t.raised, t.border, t.text),
    ] {
        state.bg_fill = fill;
        // Buttons paint `weak_bg_fill`, frames and sliders `bg_fill`.
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0, stroke);
        state.fg_stroke = Stroke::new(1.0, text);
    }
    for state in [
        &mut w.noninteractive,
        &mut w.inactive,
        &mut w.hovered,
        &mut w.active,
        &mut w.open,
    ] {
        state.corner_radius = control;
    }

    // `raised` is too close to `bg` (about 1.15:1) to see a selection in the terminal or a text field; translucent
    // `text_low` stands out on both `bg` and `surface` and keeps the text on it readable.
    v.selection.bg_fill =
        Color32::from_rgba_unmultiplied(t.text_low.r(), t.text_low.g(), t.text_low.b(), 97);
    v.selection.stroke = Stroke::new(1.0, t.text_high);
    // egui's caret is the blue selection stroke; a neutral 2 px bar matches the palette.
    v.text_cursor.stroke = Stroke::new(2.0, t.text_high);

    v.window_corner_radius = CornerRadius::same(radius::DIALOG);
    v.menu_corner_radius = CornerRadius::same(radius::DIALOG);
    v.window_stroke = Stroke::new(1.0, t.border);
    let shadow = Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 89 } else { 38 }),
    };
    v.window_shadow = shadow;
    v.popup_shadow = shadow;
    v
}

pub fn preference(choice: ThemeChoice) -> egui::ThemePreference {
    match choice {
        ThemeChoice::System => egui::ThemePreference::System,
        ThemeChoice::Light => egui::ThemePreference::Light,
        ThemeChoice::Dark => egui::ThemePreference::Dark,
    }
}

/// Installs fonts and the tokens for both themes; egui then follows the system's light/dark choice.
pub fn apply(ctx: &egui::Context) {
    // Agent output can hold any character; a missing glyph must draw a box, never stop the window.
    ctx.set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
    ctx.set_fonts(font_definitions());
    ctx.all_styles_mut(|style| {
        style.text_styles.insert(
            TextStyle::Heading,
            FontId::new(size::HEADING, medium_family()),
        );
        style.text_styles.insert(
            TextStyle::Body,
            FontId::new(size::BODY, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Button,
            FontId::new(size::BUTTON, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Small,
            FontId::new(size::SMALL, FontFamily::Proportional),
        );
        style
            .text_styles
            .insert(TextStyle::Monospace, mono(size::CODE));
    });
    ctx.set_visuals_of(egui::Theme::Dark, visuals(true));
    ctx.set_visuals_of(egui::Theme::Light, visuals(false));
}

/// Frame interval for motion while focused: egui repaints the whole window, so about 10 fps is the budget.
pub const ANIMATION_FRAME: std::time::Duration = std::time::Duration::from_millis(100);
/// egui's own transition length while animating; 0 otherwise.
pub const ANIMATION_TIME: f32 = 0.12;

/// True only while the window has focus. Every animation in the app asks this; nothing else reads focus,
/// so an unfocused or minimised window stays idle.
pub fn animate(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().focused) == Some(true)
}

/// Opacity breathing between 0.35 and 1.0 with `period` seconds. Unfocused it is the resting 1.0 and asks
/// for no repaint; focused it schedules the next frame itself, so only a visible pulse keeps the window awake.
pub fn pulse(ui: &Ui, period: f32) -> f32 {
    if !animate(ui.ctx()) {
        return 1.0;
    }
    ui.ctx().request_repaint_after(ANIMATION_FRAME);
    let t = ui.input(|i| i.time) as f32;
    0.675 + 0.325 * (std::f32::consts::TAU * t / period).cos()
}
