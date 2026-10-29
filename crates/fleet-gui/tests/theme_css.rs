//! Writes the egui theme as CSS variables for the web UI (`web/src/generated/theme.css`), so `theme.rs` stays the
//! only place a colour, size or radius is chosen. The shell's sizes also go to `web/src/generated/layout.ts`, as
//! numbers for the web's panel rules. CI fails on a stale file through its `git diff` check.
use eframe::egui::Color32;
use fleet_gui::theme::{self, layout, radius, size, space, Palette};
use std::fmt::Write;
use std::path::Path;

fn hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

fn palette_vars(p: &Palette, dark: bool) -> String {
    let t = &p.tokens;
    let tokens = [
        ("bg", t.bg),
        ("surface", t.surface),
        ("raised", t.raised),
        ("sunken", t.sunken),
        ("border", t.border),
        ("dashed", t.dashed),
        ("text-high", t.text_high),
        ("text", t.text),
        ("text-low", t.text_low),
        ("fill", t.fill),
        ("accent-text", t.accent_text),
        ("active-text", t.active_text),
        ("on-fill", t.on_fill),
        ("danger", t.danger),
    ];
    let tones = [
        ("neutral", p.tones.neutral),
        ("active", p.tones.active),
        ("attention", p.tones.attention),
        ("done", p.tones.done),
        ("muted", p.tones.muted),
        ("caution", p.tones.caution),
        ("danger", p.tones.danger),
    ];
    let alpha = theme::tint_alpha(dark) as f32 / 255.0;
    let mut out = String::new();
    for (name, c) in tokens {
        writeln!(out, "  --{name}: {};", hex(c)).unwrap();
    }
    for (name, c) in tones {
        writeln!(out, "  --tone-{name}: {};", hex(c)).unwrap();
    }
    for (name, c) in tones {
        writeln!(
            out,
            "  --tint-{name}: rgb({} {} {} / {alpha:.3});",
            c.r(),
            c.g(),
            c.b()
        )
        .unwrap();
    }
    out
}

fn px_vars(prefix: &str, values: &[(&str, f32)]) -> String {
    values
        .iter()
        .map(|(name, v)| format!("  --{prefix}-{name}: {v}px;\n"))
        .collect()
}

fn ratio_vars(prefix: &str, values: &[(&str, f32)]) -> String {
    values
        .iter()
        .map(|(name, v)| format!("  --{prefix}-{name}: {v};\n"))
        .collect()
}

const LAYOUT: [(&str, &str, f32); 9] = [
    ("header", "header", layout::HEADER),
    ("strip", "strip", layout::STRIP),
    ("column-header", "columnHeader", layout::COLUMN_HEADER),
    ("footer", "footer", layout::FOOTER),
    ("drawer", "drawer", layout::DRAWER),
    ("panel", "panel", layout::PANEL),
    ("panel-min", "panelMin", layout::PANEL_MIN),
    ("dialog", "dialog", layout::DIALOG),
    ("history", "history", layout::HISTORY),
];

const RATIOS: [(&str, &str, f32); 2] = [
    ("panel-cap", "panelCap", layout::PANEL_CAP),
    ("board-keep", "boardKeep", layout::BOARD_KEEP),
];

const SPACE: [(&str, &str, f32); 6] = [
    ("row-x", "rowX", space::ROW_X),
    ("panel-margin", "panelMargin", space::PANEL_MARGIN),
    ("picker-gap", "pickerGap", space::PICKER_GAP),
    ("button-gap", "buttonGap", space::BUTTON_GAP),
    ("nav-gap", "navGap", space::NAV_GAP),
    ("warn-room", "warnRoom", space::WARN_ROOM),
];

fn css_names(values: &[(&'static str, &str, f32)]) -> Vec<(&'static str, f32)> {
    values.iter().map(|(css, _, v)| (*css, *v)).collect()
}

fn ts_object(values: &[(&str, &'static str, f32)]) -> String {
    values
        .iter()
        .map(|(_, ts, v)| format!("{ts}: {v}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn layout_ts() -> String {
    let layout: Vec<_> = LAYOUT.iter().chain(RATIOS.iter()).copied().collect();
    format!(
        "// Generated from crates/fleet-gui/src/theme.rs by `cargo test -p fleet-gui --test theme_css`; do not edit.\n\
         export const layout = {{ {} }} as const;\n\
         export const space = {{ {} }} as const;\n",
        ts_object(&layout),
        ts_object(&SPACE)
    )
}

fn css() -> String {
    let light = palette_vars(&theme::BUILTIN.light, false);
    let dark = palette_vars(&theme::BUILTIN.dark, true);
    let dark_nested: String = dark.lines().map(|l| format!("  {l}\n")).collect();
    let sizes = px_vars(
        "size",
        &[
            ("heading", size::HEADING),
            ("body", size::BODY),
            ("title", size::TITLE),
            ("button", size::BUTTON),
            ("small", size::SMALL),
            ("meta", size::META),
            ("code", size::CODE),
            ("dot", size::DOT),
        ],
    );
    let layouts = px_vars("layout", &css_names(&LAYOUT))
        + &ratio_vars("layout", &css_names(&RATIOS))
        + &px_vars("space", &css_names(&SPACE));
    let radii = px_vars(
        "radius",
        &[
            ("control", radius::CONTROL as f32),
            ("card", radius::CARD as f32),
            ("dialog", radius::DIALOG as f32),
        ],
    );
    // The faces are the egui app's own files, so both UIs draw the same glyphs.
    let fonts = "../../../crates/fleet-gui/assets/fonts";
    format!(
        r#"/* Generated from crates/fleet-gui/src/theme.rs by `cargo test -p fleet-gui --test theme_css`; do not edit. */
@font-face {{ font-family: "IBM Plex Sans"; font-weight: 400; src: url("{fonts}/IBMPlexSans-Regular.ttf") format("truetype"); }}
@font-face {{ font-family: "IBM Plex Sans"; font-weight: 500; src: url("{fonts}/IBMPlexSans-Medium.ttf") format("truetype"); }}
@font-face {{ font-family: "IBM Plex Mono"; font-weight: 400; src: url("{fonts}/IBMPlexMono-Regular.ttf") format("truetype"); }}
:root {{
  color-scheme: light dark;
  --font-sans: "IBM Plex Sans", system-ui, sans-serif;
  --font-mono: "IBM Plex Mono", ui-monospace, monospace;
{sizes}{layouts}{radii}{light}}}
@media (prefers-color-scheme: dark) {{
  :root:not([data-theme="light"]) {{
{dark_nested}  }}
}}
:root[data-theme="dark"] {{
{dark}}}
"#
    )
}

#[test]
fn theme_css_export() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/src/generated/theme.css");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, css()).unwrap();
    std::fs::write(path.with_file_name("layout.ts"), layout_ts()).unwrap();
}

#[test]
fn the_shell_sizes_are_exported() {
    let css = css();
    assert!(css.contains("--layout-panel-min: 360px;"), "{css}");
    assert!(css.contains("--layout-panel-cap: 0.75;"), "{css}");
    assert!(css.contains("--space-row-x: 14px;"), "{css}");
    let ts = layout_ts();
    assert!(ts.contains("panelMin: 360"), "{ts}");
    assert!(ts.contains("rowX: 14"), "{ts}");
}
