//! Writes the egui theme as CSS variables for the web UI (`web/src/generated/theme.css`), so `theme.rs` stays the
//! only place a colour, size or radius is chosen. The shell's sizes also go to `web/src/generated/layout.ts`, as
//! numbers for the web's panel rules. CI fails on a stale file through its `git diff` check.
use eframe::egui::Color32;
use fleet_api::Tone;
use fleet_gui::theme::{
    self, card, column, fleet, layout, management, new_task, panel, radius, releases, size, space,
    startup, tasks, Palette,
};
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
    let counts = [
        ("neutral", Tone::Neutral),
        ("active", Tone::Active),
        ("attention", Tone::Attention),
        ("done", Tone::Done),
        ("muted", Tone::Muted),
        ("caution", Tone::Caution),
        ("danger", Tone::Danger),
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
    // A column count's colour: the tone, unless it is quiet or too faint to read on the surface.
    for (name, tone) in counts {
        writeln!(
            out,
            "  --count-{name}: {};",
            hex(theme::count_color(tone, dark))
        )
        .unwrap();
    }
    writeln!(
        out,
        "  --watermark-alpha: {};",
        theme::watermark_alpha(dark)
    )
    .unwrap();
    let (strong, soft) = theme::selection_alpha(dark);
    writeln!(out, "  --selection-alpha-strong: {strong};").unwrap();
    writeln!(out, "  --selection-alpha-soft: {soft};").unwrap();
    writeln!(out, "  --chip-alpha: {:.3};", theme::chip_alpha(dark)).unwrap();
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

const LAYOUT: [(&str, &str, f32); 23] = [
    ("header", "header", layout::HEADER),
    ("strip", "strip", layout::STRIP),
    ("column-header", "columnHeader", layout::COLUMN_HEADER),
    ("footer", "footer", layout::FOOTER),
    ("drawer", "drawer", layout::DRAWER),
    ("panel", "panel", layout::PANEL),
    ("panel-min", "panelMin", layout::PANEL_MIN),
    ("dialog", "dialog", layout::DIALOG),
    ("history", "history", layout::HISTORY),
    ("watermark", "watermark", layout::WATERMARK),
    ("watermark-bleed", "watermarkBleed", layout::WATERMARK_BLEED),
    ("menu-min", "menuMin", layout::MENU_MIN),
    (
        "release-menu-min",
        "releaseMenuMin",
        layout::RELEASE_MENU_MIN,
    ),
    ("task-list", "taskList", layout::TASK_LIST),
    ("management-list", "managementList", layout::MANAGEMENT_LIST),
    ("page-x", "pageX", layout::PAGE_X),
    ("page-y", "pageY", layout::PAGE_Y),
    ("list-row-x", "listRowX", layout::LIST_ROW_X),
    ("list-row-y", "listRowY", layout::LIST_ROW_Y),
    ("section-gap", "sectionGap", layout::SECTION_GAP),
    ("caption-gap", "captionGap", layout::CAPTION_GAP),
    ("card-padding", "cardPadding", layout::CARD_PADDING),
    ("card-gap", "cardGap", layout::CARD_GAP),
];

const RATIOS: [(&str, &str, f32); 2] = [
    ("panel-cap", "panelCap", layout::PANEL_CAP),
    ("board-keep", "boardKeep", layout::BOARD_KEEP),
];

const SPACE: [(&str, &str, f32); 10] = [
    ("row-x", "rowX", space::ROW_X),
    ("panel-margin", "panelMargin", space::PANEL_MARGIN),
    ("picker-gap", "pickerGap", space::PICKER_GAP),
    ("button-gap", "buttonGap", space::BUTTON_GAP),
    ("nav-gap", "navGap", space::NAV_GAP),
    ("warn-room", "warnRoom", space::WARN_ROOM),
    ("header-gap", "headerGap", space::HEADER_GAP),
    ("chip-gap", "chipGap", space::CHIP_GAP),
    ("item", "item", space::ITEM),
    ("summary-gap", "summaryGap", space::SUMMARY_GAP),
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

const CARD: [(&str, &str, f32); 18] = [
    ("row-title", "rowTitle", card::ROW_TITLE),
    ("row-state", "rowState", card::ROW_STATE),
    ("row-activity", "rowActivity", card::ROW_ACTIVITY),
    ("row-actions", "rowActions", card::ROW_ACTIONS),
    ("row-tally", "rowTally", card::ROW_TALLY),
    ("row-parent", "rowParent", card::ROW_PARENT),
    ("row-gap", "rowGap", card::ROW_GAP),
    ("margin-x", "marginX", card::MARGIN_X),
    ("margin-y", "marginY", card::MARGIN_Y),
    ("edge", "edge", card::EDGE),
    ("item-gap", "itemGap", card::ITEM_GAP),
    ("action-gap", "actionGap", card::ACTION_GAP),
    ("changes-dot", "changesDot", card::CHANGES_DOT),
    ("min-name", "minName", card::MIN_NAME),
    ("icon-button", "iconButton", card::ICON_BUTTON),
    ("icon-glyph", "iconGlyph", card::ICON_GLYPH),
    ("bracket-arm", "bracketArm", card::BRACKET_ARM),
    ("bracket-inset", "bracketInset", card::BRACKET_INSET),
];

const COLUMN: [(&str, &str, f32); 7] = [
    ("pad-x", "padX", column::PAD_X),
    ("gap", "gap", column::GAP),
    ("dot", "dot", column::DOT),
    ("body", "body", column::BODY),
    ("card-gap", "cardGap", column::CARD_GAP),
    ("tick-step", "tickStep", column::TICK_STEP),
    ("tick-len", "tickLen", column::TICK_LEN),
];

const FLEET: [(&str, &str, f32); 12] = [
    ("row", "row", fleet::ROW),
    ("indent", "indent", fleet::INDENT),
    ("stack-gap", "stackGap", fleet::STACK_GAP),
    ("row-gap", "rowGap", fleet::ROW_GAP),
    ("edge-room", "edgeRoom", fleet::EDGE_ROOM),
    ("dash", "dash", fleet::DASH),
    ("dash-gap", "dashGap", fleet::DASH_GAP),
    ("tally-dot", "tallyDot", fleet::TALLY_DOT),
    ("tally-gap", "tallyGap", fleet::TALLY_GAP),
    ("tally-after", "tallyAfter", fleet::TALLY_AFTER),
    ("fold-text", "foldText", fleet::FOLD_TEXT),
    ("mark", "mark", fleet::MARK),
];

const PANEL: [(&str, &str, f32); 15] = [
    ("margin", "margin", panel::MARGIN),
    ("title-top", "titleTop", panel::TITLE_TOP),
    ("title-bottom", "titleBottom", panel::TITLE_BOTTOM),
    ("band-gap", "bandGap", panel::BAND_GAP),
    ("body-y", "bodyY", panel::BODY_Y),
    ("gap-xs", "gapXs", panel::GAP_XS),
    ("gap-s", "gapS", panel::GAP_S),
    ("gap-m", "gapM", panel::GAP_M),
    ("decide-gap", "decideGap", panel::DECIDE_GAP),
    ("section-gap", "sectionGap", panel::SECTION_GAP),
    ("grid-x", "gridX", panel::GRID_X),
    ("grid-y", "gridY", panel::GRID_Y),
    ("state-dot", "stateDot", panel::STATE_DOT),
    ("output-max", "outputMax", panel::OUTPUT_MAX),
    ("icon-button", "iconButton", panel::ICON_BUTTON),
];

const NEW_TASK: [(&str, &str, f32); 9] = [
    ("width", "width", new_task::WIDTH),
    ("margin", "margin", new_task::MARGIN),
    ("head-gap", "headGap", new_task::HEAD_GAP),
    ("field-gap", "fieldGap", new_task::FIELD_GAP),
    ("caption-gap", "captionGap", new_task::CAPTION_GAP),
    ("start-gap", "startGap", new_task::START_GAP),
    ("note-gap", "noteGap", new_task::NOTE_GAP),
    ("control-inset", "controlInset", new_task::CONTROL_INSET),
    ("start-inset", "startInset", new_task::START_INSET),
];

const STARTUP: [(&str, &str, f32); 8] = [
    ("logo", "logo", startup::LOGO),
    ("bar", "bar", startup::BAR),
    ("logo-gap", "logoGap", startup::LOGO_GAP),
    ("label-gap", "labelGap", startup::LABEL_GAP),
    ("card-margin", "cardMargin", startup::CARD_MARGIN),
    ("card-gap", "cardGap", startup::CARD_GAP),
    ("card-actions", "cardActions", startup::CARD_ACTIONS),
    ("setup", "setup", startup::SETUP),
];

const TASKS: [(&str, &str, f32); 10] = [
    ("totals-y", "totalsY", tasks::TOTALS_Y),
    ("stat-gap", "statGap", tasks::STAT_GAP),
    ("list-margin", "listMargin", tasks::LIST_MARGIN),
    ("meta-gap", "metaGap", tasks::META_GAP),
    ("grid-x", "gridX", tasks::GRID_X),
    ("grid-y", "gridY", tasks::GRID_Y),
    ("log-y", "logY", tasks::LOG_Y),
    ("box", "box", tasks::BOX),
    ("box-x", "boxX", tasks::BOX_X),
    ("box-y", "boxY", tasks::BOX_Y),
];

const RELEASES: [(&str, &str, f32); 8] = [
    ("toolbar-gap", "toolbarGap", releases::TOOLBAR_GAP),
    ("form-margin", "formMargin", releases::FORM_MARGIN),
    ("title-gap", "titleGap", releases::TITLE_GAP),
    ("row-gap", "rowGap", releases::ROW_GAP),
    ("bar-width", "barWidth", releases::BAR_WIDTH),
    ("bar-height", "barHeight", releases::BAR_HEIGHT),
    ("bar-gap", "barGap", releases::BAR_GAP),
    ("move-to", "moveTo", releases::MOVE_TO),
];

const MANAGEMENT: [(&str, &str, f32); 10] = [
    ("list-margin", "listMargin", management::LIST_MARGIN),
    ("editor-width", "editorWidth", management::EDITOR_WIDTH),
    (
        "settings-width",
        "settingsWidth",
        management::SETTINGS_WIDTH,
    ),
    (
        "workflow-width",
        "workflowWidth",
        management::WORKFLOW_WIDTH,
    ),
    ("head-gap", "headGap", management::HEAD_GAP),
    ("para-gap", "paraGap", management::PARA_GAP),
    ("box-margin", "boxMargin", management::BOX_MARGIN),
    ("filter-gap", "filterGap", management::FILTER_GAP),
    ("counts-gap", "countsGap", management::COUNTS_GAP),
    ("release-gap", "releaseGap", management::RELEASE_GAP),
];

fn layout_ts() -> String {
    let layout: Vec<_> = LAYOUT.iter().chain(RATIOS.iter()).copied().collect();
    format!(
        "// Generated from crates/fleet-gui/src/theme.rs by `cargo test -p fleet-gui --test theme_css`; do not edit.\n\
         export const layout = {{ {} }} as const;\n\
         export const space = {{ {} }} as const;\n\
         export const card = {{ {}, confirmFor: {} }} as const;\n\
         export const column = {{ {} }} as const;\n\
         export const fleet = {{ {} }} as const;\n\
         export const panel = {{ {} }} as const;\n\
         export const newTask = {{ {}, promptRows: {} }} as const;\n\
         export const tasks = {{ {} }} as const;\n\
         export const releases = {{ {} }} as const;\n\
         export const management = {{ {} }} as const;\n",
        ts_object(&layout),
        ts_object(&SPACE),
        ts_object(&CARD),
        card::CONFIRM_FOR,
        ts_object(&COLUMN),
        ts_object(&FLEET),
        ts_object(&PANEL),
        ts_object(&NEW_TASK),
        new_task::PROMPT_ROWS,
        ts_object(&TASKS),
        ts_object(&RELEASES),
        ts_object(&MANAGEMENT)
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
            ("live-dot", size::LIVE_DOT),
        ],
    );
    let layouts = px_vars("layout", &css_names(&LAYOUT))
        + &ratio_vars("layout", &css_names(&RATIOS))
        + &px_vars("space", &css_names(&SPACE))
        + &px_vars("card", &css_names(&CARD))
        + &px_vars("column", &css_names(&COLUMN))
        + &px_vars("fleet", &css_names(&FLEET))
        + &px_vars("panel", &css_names(&PANEL))
        + &px_vars("new-task", &css_names(&NEW_TASK))
        + &px_vars("startup", &css_names(&STARTUP))
        + &px_vars("tasks", &css_names(&TASKS))
        + &px_vars("releases", &css_names(&RELEASES))
        + &px_vars("management", &css_names(&MANAGEMENT))
        + &format!("  --startup-turn: {}s;\n", startup::TURN);
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

#[test]
fn the_card_and_column_sizes_are_exported() {
    let css = css();
    for want in [
        "--card-row-state: 17px;",
        "--card-edge: 4px;",
        "--column-pad-x: 10px;",
        "--selection-alpha-strong: 0.26;",
        "--count-muted: ",
    ] {
        assert!(css.contains(want), "{want} in {css}");
    }
    let ts = layout_ts();
    assert!(ts.contains("confirmFor: 4"), "{ts}");
    assert!(ts.contains("rowActions: 24"), "{ts}");
}

#[test]
fn the_fleet_and_header_sizes_are_exported() {
    let css = css();
    for want in [
        "--fleet-row: 26px;",
        "--fleet-indent: 10px;",
        "--layout-menu-min: 200px;",
        "--size-live-dot: 8px;",
        "--chip-alpha: 0.122;",
    ] {
        assert!(css.contains(want), "{want} in {css}");
    }
    let ts = layout_ts();
    assert!(ts.contains("export const fleet = { row: 26"), "{ts}");
}

#[test]
fn the_panel_sizes_are_exported() {
    let css = css();
    for want in ["--panel-margin: 12px;", "--panel-output-max: 240px;"] {
        assert!(css.contains(want), "{want} in {css}");
    }
    let ts = layout_ts();
    assert!(ts.contains("export const panel = { margin: 12"), "{ts}");
}

#[test]
fn the_new_task_sizes_are_exported() {
    let css = css();
    for want in [
        "--new-task-width: 440px;",
        "--new-task-margin: 16px;",
        "--new-task-field-gap: 10px;",
    ] {
        assert!(css.contains(want), "{want} in {css}");
    }
    let ts = layout_ts();
    assert!(ts.contains("export const newTask = { width: 440"), "{ts}");
    assert!(ts.contains("promptRows: 4"), "{ts}");
}
