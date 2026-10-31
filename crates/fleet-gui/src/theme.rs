//! Colours, fonts and visuals. Nothing outside this file writes a literal colour: widgets read `tokens`,
//! and egui's stock widgets (TextEdit, ComboBox, Modal, ScrollArea) pick the same values up from `Visuals`.
use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Shadow, Stroke,
    TextStyle, Ui,
};
use fleet_api::Tone;
use fleet_client::config::{MonoFont, ThemeChoice, UiFont};
use std::sync::Arc;

#[derive(Debug, Clone, Copy)]
pub struct Tokens {
    pub bg: Color32,
    pub surface: Color32,
    pub raised: Color32,
    /// Darker than `bg`: the drawer, the terminal and the body of a column of things that are no longer running.
    pub sunken: Color32,
    pub border: Color32,
    /// Tick rules and a fleet agent's dashed outline: a step above `border`, so the dashes read on `surface`.
    pub dashed: Color32,
    pub text_high: Color32,
    pub text: Color32,
    pub text_low: Color32,
    /// The theme's accent (orange in dark, blue in light): selection, primary actions and decoration. Status never
    /// reads it; Working is `Tone::Active`. `on_fill` is a label on it.
    pub fill: Color32,
    /// The accent used as text.
    pub accent_text: Color32,
    /// `Tone::Active` (Working) used as text.
    pub active_text: Color32,
    pub on_fill: Color32,
    pub danger: Color32,
}

#[derive(Debug, Clone, Copy)]
pub struct Tones {
    pub neutral: Color32,
    pub active: Color32,
    pub attention: Color32,
    pub done: Color32,
    pub muted: Color32,
    pub caution: Color32,
    pub danger: Color32,
}

/// One half of a theme. A theme file will hold a `dark` and a `light` block of this same shape.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub tokens: Tokens,
    pub tones: Tones,
}

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub dark: Palette,
    pub light: Palette,
    pub fonts: Fonts,
}

/// The UI and mono faces, chosen independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Fonts {
    pub ui: UiFont,
    pub mono: MonoFont,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// The product theme: navy and blue with Claude orange, from the FUI design.
pub const BUILTIN: Theme = Theme {
    dark: Palette {
        tokens: Tokens {
            bg: rgb(0x070b12),
            surface: rgb(0x0b1220),
            raised: rgb(0x111b2d),
            sunken: rgb(0x05080e),
            border: rgb(0x1b2a42),
            dashed: rgb(0x24385a),
            text_high: rgb(0xe8f2ff),
            text: rgb(0xa8bbd4),
            text_low: rgb(0x7489a6),
            fill: rgb(0xd97757),
            accent_text: rgb(0xe89478),
            active_text: rgb(0x8cc8ff),
            on_fill: rgb(0x04101f),
            danger: rgb(0xff6b78),
        },
        tones: Tones {
            neutral: rgb(0x7489a6),
            active: rgb(0x3da2ff),
            attention: rgb(0xd97757),
            done: rgb(0x4fd1c5),
            muted: rgb(0x7d8aa3),
            caution: rgb(0xe8b04a),
            danger: rgb(0xff6b78),
        },
    },
    light: Palette {
        tokens: Tokens {
            bg: rgb(0xedf2f8),
            surface: rgb(0xf9fbfe),
            raised: rgb(0xeaf0f7),
            sunken: rgb(0xe4ebf4),
            border: rgb(0xcfdae8),
            dashed: rgb(0xb7c7dc),
            text_high: rgb(0x0a1627),
            text: rgb(0x2a3c56),
            text_low: rgb(0x566a86),
            fill: rgb(0x1f6fe0),
            accent_text: rgb(0x1658b8),
            active_text: rgb(0x1658b8),
            on_fill: rgb(0xffffff),
            danger: rgb(0xc62f3d),
        },
        tones: Tones {
            neutral: rgb(0x566a86),
            active: rgb(0x1f6fe0),
            attention: rgb(0xb8532f),
            done: rgb(0x0f7f73),
            muted: rgb(0x64708a),
            caution: rgb(0xa8730a),
            danger: rgb(0xc62f3d),
        },
    },
    fonts: Fonts {
        ui: UiFont::Plex,
        mono: MonoFont::Plex,
    },
};

/// The active theme's half. Only the built-in theme exists for now.
pub fn palette(dark: bool) -> &'static Palette {
    if dark {
        &BUILTIN.dark
    } else {
        &BUILTIN.light
    }
}

pub fn tokens(dark: bool) -> Tokens {
    palette(dark).tokens
}

pub fn tone_color(tone: Tone, dark: bool) -> Color32 {
    let t = &palette(dark).tones;
    match tone {
        Tone::Neutral => t.neutral,
        Tone::Active => t.active,
        Tone::Attention => t.attention,
        Tone::Done => t.done,
        Tone::Muted => t.muted,
        Tone::Caution => t.caution,
        Tone::Danger => t.danger,
    }
}

/// The colour of a column's count: quiet tones and tones too faint on `surface` for small text use `text_low`.
pub fn count_color(tone: Tone, dark: bool) -> Color32 {
    let p = tokens(dark);
    match tone {
        Tone::Muted | Tone::Neutral => p.text_low,
        tone if contrast(tone_color(tone, dark), p.surface) >= 4.5 => tone_color(tone, dark),
        _ => p.text_low,
    }
}

/// WCAG contrast ratio of two opaque colours.
fn contrast(a: Color32, b: Color32) -> f32 {
    let luminance = |c: Color32| {
        let lin = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
    };
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// The alpha of a badge background's tint; a touch stronger in dark where a tint reads fainter.
pub const fn tint_alpha(dark: bool) -> u8 {
    if dark {
        36
    } else {
        31
    }
}

/// The tone at low alpha, for badge backgrounds.
pub fn tone_tint(tone: Tone, dark: bool) -> Color32 {
    let c = tone_color(tone, dark);
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), tint_alpha(dark))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weight {
    Regular,
    Medium,
}

/// egui has no font weights; Medium is its own family holding the chosen UI font's Medium file.
pub const MEDIUM_FAMILY: &str = "ui-medium";

/// Font data keys: (regular, medium) per UI font, one per mono font.
fn ui_keys(font: UiFont) -> (&'static str, &'static str) {
    match font {
        UiFont::Plex => ("plex-sans", "plex-sans-medium"),
        UiFont::SpaceGrotesk => ("space-grotesk", "space-grotesk-medium"),
        UiFont::Atkinson => ("atkinson-next", "atkinson-next-medium"),
    }
}

fn mono_key(font: MonoFont) -> &'static str {
    match font {
        MonoFont::Plex => "plex-mono",
        MonoFont::JetBrains => "jetbrains-mono",
        MonoFont::Atkinson => "atkinson-mono",
    }
}

/// A family holding only that UI font's regular (then egui's fallbacks), so Settings can show each name in its
/// own face whichever font is active.
pub fn ui_family(font: UiFont) -> FontFamily {
    FontFamily::Name(format!("ui:{}", ui_keys(font).0).into())
}

/// As `ui_family`, for a mono font.
pub fn mono_family(font: MonoFont) -> FontFamily {
    FontFamily::Name(format!("mono:{}", mono_key(font)).into())
}

pub mod size {
    pub const HEADING: f32 = 15.0;
    pub const BODY: f32 = 13.0;
    pub const TITLE: f32 = 12.5;
    pub const BUTTON: f32 = 12.0;
    pub const SMALL: f32 = 11.5;
    pub const META: f32 = 10.5;
    pub const CODE: f32 = 11.5;
    /// A status dot beside a line of text, as in the footer.
    pub const DOT: f32 = 7.0;
    /// The drawer footer's connection dot.
    pub const LIVE_DOT: f32 = 8.0;
}

/// The app's name wherever a person reads it. Ids, paths and binaries keep `fleet-manager`.
pub const APP_NAME: &str = "Fleet Manager";

/// The shell's fixed sizes. The drawer's header and footer share the content's heights so their border lines run
/// unbroken across the window.
pub mod layout {
    pub const HEADER: f32 = 40.0;
    /// The thin row under the board's header: the task counts now, view controls later.
    pub const STRIP: f32 = 28.0;
    /// The board's column headers and the detail panel's top row, so their bottom lines run as one.
    pub const COLUMN_HEADER: f32 = 36.0;
    pub const FOOTER: f32 = 28.0;
    pub const DRAWER: f32 = 200.0;
    /// The detail panel's width when it first opens.
    pub const PANEL: f32 = 420.0;
    /// The narrowest a drag can make the detail panel: a terminal stays readable.
    pub const PANEL_MIN: f32 = 360.0;
    /// The share of the board the detail panel may take before a drag stops.
    pub const PANEL_CAP: f32 = 0.75;
    /// The detail panel goes fullscreen once the board would keep less than this share of its width.
    pub const BOARD_KEEP: f32 = 0.25;
    /// A dialog's widest.
    pub const DIALOG: f32 = 520.0;
    /// The Archive dialog's widest: its plan lists paths.
    pub const ARCHIVE_DIALOG: f32 = 560.0;
    /// The footer's History popup.
    pub const HISTORY: f32 = 380.0;
    /// The drawer's watermark: its side, and how far it hangs out past the drawer's left edge.
    pub const WATERMARK: f32 = 240.0;
    pub const WATERMARK_BLEED: f32 = 96.0;
    /// The repo and persona filters' menus at their narrowest.
    pub const MENU_MIN: f32 = 200.0;
    /// The release filter's menu at its narrowest.
    pub const RELEASE_MENU_MIN: f32 = 160.0;
    /// The Tasks view's list; the passport takes the rest.
    pub const TASK_LIST: f32 = 250.0;
    /// Management's lists (definitions, personas, products); the detail takes the rest.
    pub const MANAGEMENT_LIST: f32 = 260.0;
    /// A page's side and top margins, which every view's body keeps.
    pub const PAGE_X: f32 = 22.0;
    pub const PAGE_Y: f32 = 18.0;
    /// A selectable list row's inner margin (`list_row`).
    pub const LIST_ROW_X: f32 = 8.0;
    pub const LIST_ROW_Y: f32 = 5.0;
    /// Above a section's mono caption, and under it.
    pub const SECTION_GAP: f32 = 16.0;
    pub const CAPTION_GAP: f32 = 6.0;
    /// A page card (a release, a product on Overview): its inner margin, and the gap between cards.
    pub const CARD_PADDING: f32 = 12.0;
    pub const CARD_GAP: f32 = 10.0;
    /// The web only: a scrollbar's thickness, drawn so it stays visible.
    pub const SCROLLBAR: f32 = 16.0;
}

/// The startup screens: the intro, and the card the failure and the first-run steps sit on.
pub mod startup {
    /// The intro's turning logo.
    pub const LOGO: f32 = 160.0;
    /// Seconds per turn of the logo's radar.
    pub const TURN: f32 = 4.0;
    pub const BAR: f32 = 320.0;
    /// Logo to bar, and bar to label.
    pub const LOGO_GAP: f32 = 24.0;
    pub const LABEL_GAP: f32 = 8.0;
    /// The app's boot mark: its ring to the bar (the web only; egui keeps `LOGO_GAP`).
    pub const MARK_GAP: f32 = 48.0;
    /// The card's inner margin, the gap between its lines, and the gap before its buttons.
    pub const CARD_MARGIN: f32 = 16.0;
    pub const CARD_GAP: f32 = 6.0;
    pub const CARD_ACTIONS: f32 = 10.0;
    /// The persona setup's width.
    pub const SETUP: f32 = 480.0;
}

/// The drawer watermark's opacity: the accent at this fraction.
pub fn watermark_alpha(dark: bool) -> f32 {
    if dark {
        0.09
    } else {
        0.08
    }
}

/// A selected item's wash: its strength at the right edge and 0.45 in, which fades out leftward.
pub fn selection_alpha(dark: bool) -> (f32, f32) {
    if dark {
        (0.26, 0.08)
    } else {
        (0.18, 0.05)
    }
}

/// A selected chip's fill: the accent at this fraction.
pub fn chip_alpha(dark: bool) -> f32 {
    if dark {
        31.0 / 255.0
    } else {
        23.0 / 255.0
    }
}

/// A board card's sizes, shared with the web UI like `layout`. Every card has every row, filled or not, so all cards
/// are the same size whatever their column or content.
pub mod card {
    use super::size;
    /// One line of title, at the title's line height, and the time in status.
    pub const ROW_TITLE: f32 = size::TITLE * 1.35;
    /// The state badge, the changes dot, the repo and the size.
    pub const ROW_STATE: f32 = 17.0;
    pub const ROW_ACTIVITY: f32 = 16.0;
    /// A text button's minimum height, for an action the client has no icon for.
    pub const ROW_ACTIONS: f32 = 24.0;
    /// A task card's fleet tally and its fold toggle.
    pub const ROW_TALLY: f32 = 17.0;
    /// A full fleet card's `↳ <task>` line.
    pub const ROW_PARENT: f32 = 15.0;
    pub const ROW_GAP: f32 = 4.0;
    pub const MARGIN_X: f32 = 9.0;
    pub const MARGIN_Y: f32 = 8.0;
    /// The status edge's width: wide enough to read at a glance.
    pub const EDGE: f32 = 4.0;
    /// Between items on a row.
    pub const ITEM_GAP: f32 = 6.0;
    /// Between the action buttons.
    pub const ACTION_GAP: f32 = 2.0;
    /// The has-changes dot.
    pub const CHANGES_DOT: f32 = 6.0;
    /// The least of a repo name worth showing beside its "+N": a letter and the "…".
    pub const MIN_NAME: f32 = 16.0;
    /// A small icon button's side, and its glyph's half-size.
    pub const ICON_BUTTON: f32 = 20.0;
    pub const ICON_GLYPH: f32 = 4.5;
    /// The selected card's corner brackets: arm length and inset.
    pub const BRACKET_ARM: f32 = 7.0;
    pub const BRACKET_INSET: f32 = 3.0;
    /// Seconds a destructive action stays armed for its second click.
    pub const CONFIRM_FOR: f64 = 4.0;
}

/// A board column's sizes, shared with the web UI like `layout`.
pub mod column {
    pub const PAD_X: f32 = 10.0;
    /// Between the header's dot, title and count.
    pub const GAP: f32 = 7.0;
    pub const DOT: f32 = 7.0;
    /// Around the cards.
    pub const BODY: f32 = 8.0;
    pub const CARD_GAP: f32 = 6.0;
    /// The header's dashed ticks: one every `TICK_STEP`, `TICK_LEN` long, up from its bottom line.
    pub const TICK_STEP: f32 = 10.0;
    pub const TICK_LEN: f32 = 3.0;
}

/// The detail panel's sizes (preview.rs, breakdown.rs, repo_request.rs), shared with the web UI like `layout`.
pub mod panel {
    /// Around the header row, the bands and the bottom box.
    pub const MARGIN: f32 = 12.0;
    /// Above and below the title block.
    pub const TITLE_TOP: f32 = 10.0;
    pub const TITLE_BOTTOM: f32 = 6.0;
    /// Under the link, details and repos bands; above the reply box's row.
    pub const BAND_GAP: f32 = 8.0;
    /// The tab body's top and bottom padding.
    pub const BODY_Y: f32 = 10.0;
    /// The breakdown's, the repo request's and the forms' small spaces.
    pub const GAP_XS: f32 = 2.0;
    pub const GAP_S: f32 = 4.0;
    pub const GAP_M: f32 = 6.0;
    /// Above the breakdown's decision, and above its gate.
    pub const DECIDE_GAP: f32 = 10.0;
    pub const SECTION_GAP: f32 = 12.0;
    /// The breakdown table's column and row gaps.
    pub const GRID_X: f32 = 10.0;
    pub const GRID_Y: f32 = 4.0;
    /// A step's state dot.
    pub const STATE_DOT: f32 = 6.0;
    /// The failed gate step's output, at its tallest.
    pub const OUTPUT_MAX: f32 = 240.0;
    /// An icon button in a header row (Close, Fullscreen, Open in terminal).
    pub const ICON_BUTTON: f32 = 26.0;
}

/// The New task dialog's sizes (new_task.rs), shared with the web UI like `layout`.
pub mod new_task {
    /// The dialog's outside width.
    pub const WIDTH: f32 = 440.0;
    /// Every modal's inner margin (`modal_frame`).
    pub const MARGIN: f32 = 16.0;
    /// Under the heading row, under the repos picker, and after the fleet section.
    pub const HEAD_GAP: f32 = 12.0;
    /// Between one field and the next caption.
    pub const FIELD_GAP: f32 = 10.0;
    /// Between a caption and its field, and above a refusal's setup link.
    pub const CAPTION_GAP: f32 = 4.0;
    /// After each repo's start choice.
    pub const START_GAP: f32 = 6.0;
    /// Under the refusal or the error line.
    pub const NOTE_GAP: f32 = 8.0;
    /// What a dropdown leaves of the content width.
    pub const CONTROL_INSET: f32 = 24.0;
    /// What a checkout dropdown leaves of the dialog's width.
    pub const START_INSET: f32 = 64.0;
    /// The goal box's height, in rows.
    pub const PROMPT_ROWS: usize = 4;
}

/// The Tasks view's spacing, shared with the web UI like `layout`.
pub mod tasks {
    /// The totals row's top and bottom margin.
    pub const TOTALS_Y: f32 = 10.0;
    /// After each stat in the totals.
    pub const STAT_GAP: f32 = 28.0;
    /// Around the list.
    pub const LIST_MARGIN: f32 = 8.0;
    /// Under the passport's title row.
    pub const META_GAP: f32 = 4.0;
    /// The passport's grids: between columns, and between rows (the log's are tighter).
    pub const GRID_X: f32 = 16.0;
    pub const GRID_Y: f32 = 4.0;
    pub const LOG_Y: f32 = 2.0;
    /// The Diff and Last screen boxes: their height (they scroll inside the passport, which scrolls too) and margin.
    pub const BOX: f32 = 280.0;
    pub const BOX_X: f32 = 10.0;
    pub const BOX_Y: f32 = 8.0;
}

/// Settings → General's daemon sections and the Daemon tab.
pub mod settings {
    /// The Daemon tab's grid: between its name and value columns, and between rows.
    pub const DAEMON_GRID_X: f32 = 16.0;
    pub const DAEMON_GRID_Y: f32 = 6.0;
    /// Above Restart daemon.
    pub const DAEMON_GAP: f32 = 12.0;
    /// A folder row at most: wide enough for a real path, not the whole window on a wide screen.
    pub const FOLDER_ROW: f32 = 640.0;
    /// A limit's field.
    pub const LIMIT_FIELD: f32 = 70.0;
    /// Attribution's and PR labels' fields.
    pub const PR_FIELD: f32 = 320.0;
    /// Above Extras' Check again.
    pub const EXTRAS_GAP: f32 = 12.0;
    /// Above the pending-restart line, under the Personas folder (not a section's 16).
    pub const PENDING_GAP: f32 = 8.0;
    /// The relaunched wizard's widest.
    pub const WIZARD_WIDTH: f32 = 560.0;
    /// The Log tab's Search field.
    pub const LOG_SEARCH: f32 = 220.0;
    /// A log row's level dot.
    pub const LOG_DOT: f32 = 7.0;
    /// A log row's height: egui's interact height, which a mono code line stays within.
    pub const LOG_ROW: f32 = 18.0;
}

/// Management's spacing, shared with the web UI like `layout`.
pub mod management {
    /// Around a tab's list.
    pub const LIST_MARGIN: f32 = 12.0;
    /// An editor's column (Instructions, a definition's file), and Agent settings' narrower one.
    pub const EDITOR_WIDTH: f32 = 760.0;
    pub const SETTINGS_WIDTH: f32 = 620.0;
    /// A persona's pane in the Personas tab.
    pub const PERSONA_WIDTH: f32 = 520.0;
    /// A specialist nickname's field.
    pub const NICKNAME_WIDTH: f32 = 220.0;
    /// A product's pane in the Products tab, and the New product dialog.
    pub const PRODUCT_WIDTH: f32 = 760.0;
    pub const PRODUCT_DIALOG_WIDTH: f32 = 560.0;
    /// An override row's pickers and its free-text value.
    pub const OVERRIDE_SECTION_WIDTH: f32 = 90.0;
    pub const OVERRIDE_KEY_WIDTH: f32 = 170.0;
    pub const OVERRIDE_VALUE_WIDTH: f32 = 150.0;
    pub const OVERRIDE_TEXT_WIDTH: f32 = 110.0;
    /// Internal resources: the file list's and the file text's most height before they scroll.
    pub const INTERNAL_FILES_HEIGHT: f32 = 160.0;
    pub const INTERNAL_TEXT_HEIGHT: f32 = 320.0;
    /// The Workflow tab's text column.
    pub const WORKFLOW_WIDTH: f32 = 560.0;
    /// Under a tab's heading, and between its paragraphs.
    pub const HEAD_GAP: f32 = 8.0;
    pub const PARA_GAP: f32 = 12.0;
    /// Around a boxed example.
    pub const BOX_MARGIN: f32 = 10.0;
    /// Overview: under the filter row, and between a product's name and its counts.
    pub const FILTER_GAP: f32 = 10.0;
    pub const COUNTS_GAP: f32 = 12.0;
    /// Overview: above each release row.
    pub const RELEASE_GAP: f32 = 6.0;
}

/// The Releases view's spacing, shared with the web UI like `layout`.
pub mod releases {
    /// Under New release or the form.
    pub const TOOLBAR_GAP: f32 = 8.0;
    /// The form's inner margin.
    pub const FORM_MARGIN: f32 = 10.0;
    /// Under a card's title row, and above its buttons and its tasks.
    pub const TITLE_GAP: f32 = 4.0;
    pub const ROW_GAP: f32 = 6.0;
    /// A progress bar, and the gap between a card's two bars.
    pub const BAR_WIDTH: f32 = 140.0;
    pub const BAR_HEIGHT: f32 = 6.0;
    pub const BAR_GAP: f32 = 16.0;
    /// The Move to… combo's width.
    pub const MOVE_TO: f32 = 90.0;
}

/// An orchestrated task's fleet on the board, shared with the web UI like `layout`.
pub mod fleet {
    /// A Compact fleet row's height.
    pub const ROW: f32 = 26.0;
    /// The fleet sits a little in from its task card, so a group reads as one.
    pub const INDENT: f32 = 10.0;
    /// Between the fleet's rows or cards.
    pub const STACK_GAP: f32 = 4.0;
    /// Between items on a fleet row.
    pub const ROW_GAP: f32 = 6.0;
    /// Kept clear at a fleet row's left for its status edge.
    pub const EDGE_ROOM: f32 = 4.0;
    /// The dashed outline of a fleet card or row: dash and gap.
    pub const DASH: f32 = 3.0;
    pub const DASH_GAP: f32 = 2.0;
    /// The task card's tally: the dot, the gap to its count, and the space after it.
    pub const TALLY_DOT: f32 = 6.0;
    pub const TALLY_GAP: f32 = 3.0;
    pub const TALLY_AFTER: f32 = 4.0;
    /// The fold toggle's room for its triangle, before the text.
    pub const FOLD_TEXT: f32 = 12.0;
    /// A mark's side (orchestrator, specialist, PR, warning).
    pub const MARK: f32 = 14.0;
}

/// The shell's gaps, shared with the web UI like `layout`.
pub mod space {
    /// The side padding of the drawer's and the content's rows.
    pub const ROW_X: f32 = 14.0;
    /// The drawer's padding around its pickers and nav.
    pub const PANEL_MARGIN: f32 = 12.0;
    /// Under each drawer picker.
    pub const PICKER_GAP: f32 = 8.0;
    /// Between New product and New task.
    pub const BUTTON_GAP: f32 = 6.0;
    /// Above the view list.
    pub const NAV_GAP: f32 = 10.0;
    /// Kept beside the persona picker for the ⚠ painted after a broken login.
    pub const WARN_ROOM: f32 = 22.0;
    /// Around the Kanban header's filters.
    pub const HEADER_GAP: f32 = 6.0;
    /// Between the Fleet switch's chips.
    pub const CHIP_GAP: f32 = 2.0;
    /// egui's default gap between items on a row, which the header and the drawer footer keep.
    pub const ITEM: f32 = 8.0;
    /// The extra room before the drawer footer's agent summary.
    pub const SUMMARY_GAP: f32 = 4.0;
}

pub mod radius {
    pub const CONTROL: u8 = 3;
    pub const CARD: u8 = 4;
    pub const DIALOG: u8 = 6;
}

fn medium_family() -> FontFamily {
    FontFamily::Name(MEDIUM_FAMILY.into())
}

/// The UI font at `size`. Laying out an unregistered named family panics, and `apply` is not run by widget
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

const FONT_FILES: [(&str, &[u8]); 9] = [
    (
        "plex-sans",
        include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf"),
    ),
    (
        "plex-sans-medium",
        include_bytes!("../assets/fonts/IBMPlexSans-Medium.ttf"),
    ),
    (
        "plex-mono",
        include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf"),
    ),
    (
        "space-grotesk",
        include_bytes!("../assets/fonts/SpaceGrotesk-Regular.ttf"),
    ),
    (
        "space-grotesk-medium",
        include_bytes!("../assets/fonts/SpaceGrotesk-Medium.ttf"),
    ),
    (
        "jetbrains-mono",
        include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
    ),
    (
        "atkinson-next",
        include_bytes!("../assets/fonts/AtkinsonHyperlegibleNext-Regular.ttf"),
    ),
    (
        "atkinson-next-medium",
        include_bytes!("../assets/fonts/AtkinsonHyperlegibleNext-Medium.ttf"),
    ),
    (
        "atkinson-mono",
        include_bytes!("../assets/fonts/AtkinsonHyperlegibleMono-Regular.ttf"),
    ),
];

fn font_definitions(fonts: Fonts) -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    for (key, bytes) in FONT_FILES {
        defs.font_data
            .insert(key.into(), Arc::new(FontData::from_static(bytes)));
    }

    // egui's own fonts stay behind ours: they cover the glyphs (emoji, symbols, CJK) the bundled faces lack.
    let proportional = defs.families[&FontFamily::Proportional].clone();
    let monospace = defs.families[&FontFamily::Monospace].clone();
    let first = |key: &str, rest: &[String]| {
        std::iter::once(key.to_owned())
            .chain(rest.iter().cloned())
            .collect::<Vec<_>>()
    };
    for font in UiFont::ALL {
        defs.families
            .insert(ui_family(font), first(ui_keys(font).0, &proportional));
    }
    for font in MonoFont::ALL {
        defs.families
            .insert(mono_family(font), first(mono_key(font), &monospace));
    }
    let (regular, medium) = ui_keys(fonts.ui);
    defs.families
        .insert(medium_family(), first(medium, &proportional));
    defs.families
        .insert(FontFamily::Proportional, first(regular, &proportional));
    defs.families.insert(
        FontFamily::Monospace,
        first(mono_key(fonts.mono), &monospace),
    );
    defs
}

/// Re-installs the fonts with `fonts` active; egui lays text out again from the next frame.
pub fn set_fonts(ctx: &egui::Context, fonts: Fonts) {
    ctx.set_fonts(font_definitions(fonts));
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

    // The accent marks what the person picked. These alphas keep it visible on `bg` and `surface` (1.5:1) and the
    // text on it readable (4.5:1); the passing range is about 74..115 in dark and 79..146 in light.
    let alpha = if dark { 92 } else { 108 };
    v.selection.bg_fill =
        Color32::from_rgba_unmultiplied(t.fill.r(), t.fill.g(), t.fill.b(), alpha);
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
    ctx.set_fonts(font_definitions(BUILTIN.fonts));
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
/// Frame interval for a card edge's sweep: at `ANIMATION_FRAME` the band visibly jumps along the edge. Still gated
/// like every animation, so it costs nothing unless a Working card is on a focused window.
pub const SWEEP_FRAME: std::time::Duration = std::time::Duration::from_millis(40);
/// egui's own transition length while animating; 0 otherwise.
pub const ANIMATION_TIME: f32 = 0.12;

/// True only while the window has focus and the Animations setting is on. Every animation in the app asks this;
/// nothing else reads focus, so an unfocused or minimised window stays idle. A scripted run forces it on, over the
/// setting too: under Xvfb with no window manager the viewport never reports focus, and the recording would show a
/// frozen board.
pub fn animate(ctx: &egui::Context) -> bool {
    let flag = |key| ctx.data(|d| d.get_temp::<bool>(egui::Id::unique(key)));
    flag(FORCE_ANIMATION).unwrap_or(false)
        || (flag(ANIMATIONS).unwrap_or(true) && ctx.input(|i| i.viewport().focused) == Some(true))
}

const FORCE_ANIMATION: &str = "fleet-force-animation";
const ANIMATIONS: &str = "fleet-animations";

/// The Animations setting; read by `animate`, so every animation stops with it.
pub fn set_animations(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique(ANIMATIONS), on));
}

/// Makes `animate` true from now on, focused or not.
pub fn force_animation(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique(FORCE_ANIMATION), true));
}

/// Where a highlight band is on its way down a card's edge, 0..1 over `period` seconds, or `None` when
/// animation is off. Unfocused it asks for no repaint, so only a visible sweep keeps the window awake.
pub fn sweep(ui: &Ui, period: f32) -> Option<f32> {
    if !animate(ui.ctx()) {
        return None;
    }
    ui.ctx().request_repaint_after(SWEEP_FRAME);
    let t = ui.input(|i| i.time) as f32;
    Some((t / period).fract())
}

/// Seconds since start, with a repaint asked for `frame` from now, or `None` when animation is off.
fn clock(ui: &Ui, frame: std::time::Duration) -> Option<f32> {
    if !animate(ui.ctx()) {
        return None;
    }
    ui.ctx().request_repaint_after(frame);
    Some(ui.input(|i| i.time) as f32)
}

fn ease_in_out(x: f32) -> f32 {
    0.5 - 0.5 * (std::f32::consts::PI * x).cos()
}

/// 0 → 1 → 0 over `period` seconds, eased; a breathing edge (Needs you).
pub fn pulse_at(t: f32, period: f32) -> f32 {
    0.5 - 0.5 * (std::f32::consts::TAU * (t / period).rem_euclid(1.0)).cos()
}

/// A band's position 0..1 that sweeps down in `period` seconds and back up in the next, eased at both ends; the
/// Working edge's visor.
pub fn visor_at(t: f32, period: f32) -> f32 {
    let leg = (t / period).rem_euclid(2.0);
    ease_in_out(if leg <= 1.0 { leg } else { 2.0 - leg })
}

pub fn pulse(ui: &Ui, period: f32) -> Option<f32> {
    clock(ui, SWEEP_FRAME).map(|t| pulse_at(t, period))
}

pub fn visor(ui: &Ui, period: f32) -> Option<f32> {
    clock(ui, SWEEP_FRAME).map(|t| visor_at(t, period))
}

/// 0..1 linear over `period` seconds: the radar's turn. Slow, so the general frame budget is smooth enough.
pub fn spin(ui: &Ui, period: f32) -> Option<f32> {
    clock(ui, ANIMATION_FRAME).map(|t| (t / period).rem_euclid(1.0))
}

/// Like `spin`, for a fast mover that crosses the whole window: at `ANIMATION_FRAME` it would jump tens of pixels
/// per frame, so it repaints at `SWEEP_FRAME`.
pub fn sweep_spin(ui: &Ui, period: f32) -> Option<f32> {
    clock(ui, SWEEP_FRAME).map(|t| (t / period).rem_euclid(1.0))
}

/// Three layers of `color` for a fake glow (egui has no blur), widest and faintest first. Fainter in light, where a
/// glow reads as a smudge.
pub fn glow(color: Color32, dark: bool) -> [Color32; 3] {
    let alphas: [u8; 3] = if dark { [20, 41, 77] } else { [10, 20, 38] };
    alphas.map(|a| Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), a))
}

/// The bright band that runs along a working card's edge: `Tone::Active` pulled toward white so it reads as light
/// passing over the line.
pub fn visor_band(dark: bool) -> Color32 {
    tone_color(Tone::Active, dark).lerp_to_gamma(Color32::WHITE, 0.55)
}
