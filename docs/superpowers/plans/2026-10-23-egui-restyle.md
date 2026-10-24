# egui Restyle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restyle `fleet-gui` into a polished light and dark theme in vibe-kanban's visual language. The window gets one slim top bar with a repo dropdown, a theme switch and a New agent dialog. The board, cards and side panel are restyled.

**Architecture:**
- All colours, sizes, radii and fonts live in `crates/fleet-gui/src/theme.rs`. `theme::apply` installs them into egui `Visuals` and `Style` for both themes, so stock widgets match without per-call styling.
- The primitives in `widgets/` are the only place buttons, badges, tabs, icons and dots are drawn. Components compose them.
- The theme choice is persisted in `GuiSettings` in `fleet-client`.

**Tech Stack:**
- Rust 2021, eframe/egui 0.37 (glow);
- `egui_kittest` 0.37 for UI tests;
- IBM Plex Sans/Mono TTFs vendored with `include_bytes!`.

**Spec:** `docs/superpowers/specs/2026-10-23-egui-restyle-design.md`. The mockup is `docs/superpowers/specs/2026-10-23-egui-restyle-mockup.html`; open it in a browser. Read both before any task.

## Global Constraints

- Scope is `crates/fleet-gui`, plus `GuiSettings` in `crates/fleet-client/src/config.rs`. No change to `fleet-api`, the daemon, the HTTP API or `web/`.
- Dependencies stay as they are: egui/eframe 0.37 with the glow renderer and no new crates.
- Fonts are compiled in with `include_bytes!`, so nothing is fetched at runtime and the binary runs air-gapped.
- `MissingGlyphPolicy::Tofu` stays. egui's default fonts stay as fallbacks behind Plex.
- No literal colour (`Color32::from_rgb`, `Color32::WHITE`, hex) outside `theme.rs`.
- Visible labels stay unchanged except these renames from the spec:
  - the card's input-action button becomes "Send";
  - the repo pills become a "Repo" dropdown;
  - the spawn strip moves into a "New agent" dialog;
  - the preview's Close becomes a ✕ icon whose accessible label is still "Close".
- **The window stays idle unless focused.** No animation and no `request_repaint` loop beyond the existing 15 s `REFRESH`, events and background results. The only exception is motion gated by `theme::animate` (Task 10), throttled to about 100 ms and only while the window has focus. `egui_kittest`'s `Harness::run()` panics on continuous repaint requests.
- **Spec values to use verbatim:**
  - the token table: `bg`, `surface`, `raised`, `border`, `text_high`, `text`, `text_low`, `fill`/`on_fill`, `danger`;
  - the tone colours;
  - radii: 3 for buttons, inputs and badges; 4 for cards; 6 for dialogs and menus;
  - type sizes: heading 15, body 13, card title 12.5, button 12, small 11.5, meta 10.5, code 11.5.
- **No `git commit` steps.** Antonio commits by hand. Each task ends with its tests passing and the tree left uncommitted.

## Review Focus

1. **The theme switch and "Remember my choice" both write `gui.toml`.** Saving one must keep the other's value. Today `CloseFlow` holds a `GuiSettings` loaded at startup and saves it whole, which would wipe a theme chosen later. Tests are in Task 2 and Task 8.
2. **A long unbroken task or branch** (300 characters with no spaces) must stay inside its card and must not widen its column. The task is cut to two lines, the branch to one, and the full text is on hover. Test is in Task 4.
3. **Ctrl+Enter starts the agent without leaving a newline** in the prompt, and plain Enter still inserts one. Test is in Task 6.
4. **A spawn that fails after the dialog was closed** shows in the app-level danger line. A spawn that succeeds after the dialog was closed does not reopen it. Test is in Task 6.
5. **Widgets drawn without `theme::apply`** (every widget-level test, and the first frame before fonts load) must not panic on the missing Medium family. Test is in Task 1.

---

### Task 1: Fonts, tokens and visuals in `theme.rs`

**Files:**
- Create: `crates/fleet-gui/assets/fonts/`:
  - `IBMPlexSans-Regular.ttf` and `IBMPlexSans-Medium.ttf`, from the zip at `https://github.com/IBM/plex/releases/download/%40ibm/plex-sans%401.1.0/ibm-plex-sans.zip`, path `ibm-plex-sans/fonts/complete/ttf/`;
  - `IBMPlexMono-Regular.ttf`, from `https://github.com/IBM/plex/releases/download/%40ibm/plex-mono%401.1.0/ibm-plex-mono.zip`, path `ibm-plex-mono/fonts/complete/ttf/`;
  - `LICENSE.txt`, from `ibm-plex-sans/LICENSE.txt` (SIL OFL 1.1, the same text for both families).
- Modify: `crates/fleet-gui/src/theme.rs` (rewrite).
- Modify: every caller of `palette(…)`. This is a mechanical rename: `palette`→`tokens`, `.muted`→`.text_low`. The callers are `app.rs`, `components/{card,column,preview,spawn_form}.rs` and `widgets/{badge,button,field}.rs`.
- Test: `crates/fleet-gui/tests/widgets.rs`.

**Interfaces:**
- Produces:

```rust
pub struct Tokens { pub bg: Color32, pub surface: Color32, pub raised: Color32, pub border: Color32,
    pub text_high: Color32, pub text: Color32, pub text_low: Color32, pub fill: Color32, pub on_fill: Color32,
    pub danger: Color32 }
pub fn tokens(dark: bool) -> Tokens;
pub fn tone_color(tone: Tone, dark: bool) -> Color32;          // values unchanged
pub fn tone_tint(tone: Tone, dark: bool) -> Color32;           // tone at ~14% alpha dark, ~12% light
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum Weight { Regular, Medium }
pub const MEDIUM_FAMILY: &str = "plex-medium";
pub fn sans(ui: &Ui, size: f32, weight: Weight) -> FontId;     // Medium falls back to Proportional when MEDIUM_FAMILY is not installed
pub fn mono(size: f32) -> FontId;
pub mod size { pub const HEADING: f32 = 15.0; pub const BODY: f32 = 13.0; pub const TITLE: f32 = 12.5;
    pub const BUTTON: f32 = 12.0; pub const SMALL: f32 = 11.5; pub const META: f32 = 10.5; pub const CODE: f32 = 11.5; }
pub mod radius { pub const CONTROL: u8 = 3; pub const CARD: u8 = 4; pub const DIALOG: u8 = 6; }
pub fn apply(ctx: &egui::Context);   // fonts + both themes' Visuals + text styles + Tofu
```

- [ ] **Step 1: Write the failing tests** in `tests/widgets.rs`:
  - **Delete** `web_tokens`, `tone_colors_match_the_web_tokens` and `the_palette_has_distinct_light_and_dark_surfaces_and_the_danger_colour`.
  - **`tokens_match_the_spec_table`:** assert `hex(tokens(true).X)` and `hex(tokens(false).X)` for all ten tokens, using the spec table values. For example, `tokens(true).bg == "#1c1c1c"`, `tokens(false).surface == "#ffffff"`, `tokens(true).danger == "#e5534b"` and `tokens(false).danger == "#c4302b"`.
  - **`light_and_dark_differ_where_they_must`:** `bg`, `surface`, `text_high` and `fill` differ between `tokens(true)` and `tokens(false)`.
  - **`apply_installs_plex_first_and_keeps_egui_fonts_as_fallback`:**
    1. `let ctx = egui::Context::default(); theme::apply(&ctx);`
    2. Run two empty frames (`let _ = ctx.run_ui(Default::default(), |_| {});`, or whatever 0.37's frame call is), because fonts install at frame start.
    3. Assert that `ctx.fonts(|f| f.families())` contains `FontFamily::Name(MEDIUM_FAMILY.into())`.
    4. Assert that the Proportional list in `ctx.fonts(|f| f.definitions().families[&FontFamily::Proportional].clone())` starts with the Plex Sans key and still contains egui's default keys after it.
    5. Assert the same for the Monospace list with Plex Mono.
  - **`medium_falls_back_without_apply`:** inside `Harness::new_ui`, `theme::sans(ui, 12.0, Weight::Medium).family == FontFamily::Proportional`.

- [ ] **Step 2: Run them and see them fail.**
  Run: `cargo test -p fleet-gui --test widgets`
  Expected: compile errors (`tokens`, `sans` and `MEDIUM_FAMILY` not found).

- [ ] **Step 3: Implement `theme.rs` per the Interfaces block.**
  - **Fonts:**
    - Register Plex Sans Regular first in `Proportional`, Plex Mono first in `Monospace`, and Plex Sans Medium as the sole member of a new `FontFamily::Name(MEDIUM_FAMILY)`, followed by the Proportional fallbacks.
    - Install with `ctx.add_font` or `set_fonts` on top of `FontDefinitions::default()`.
  - **Text styles:**
    - `Heading` = `sans(MEDIUM, HEADING)`, built directly with `FontId::new(size::HEADING, FontFamily::Name(...))`, since apply has just registered it;
    - `Body` = 13, `Button` = 12, `Small` = 11.5, `Monospace` = 11.5 Plex Mono;
    - set via `ctx.all_styles_mut`.
  - **Visuals per theme, from `tokens`:**
    - `panel_fill` = `bg`, `window_fill` = `surface`, `extreme_bg_color` = `bg` (TextEdit background), `faint_bg_color` = `raised`;
    - `widgets.noninteractive.bg_stroke` = `border`;
    - `inactive`: fill `surface`, stroke `border`;
    - `hovered`: fill `raised`, stroke `text_low`;
    - `active`: fill `raised`;
    - `open`: fill `raised`;
    - every widget state's `corner_radius` = `radius::CONTROL`;
    - `fg_stroke` colours: `text` normally, `text_high` when hovered or active;
    - `selection.bg_fill` = `raised`, `selection.stroke` = `text_high`;
    - `window_corner_radius` and `menu_corner_radius` = `radius::DIALOG`;
    - `window_stroke` = `border`;
    - `window_shadow` and `popup_shadow`: small and soft, about 16 px blur with black at 35% alpha in dark and 15% in light;
    - `override_text_color` = `None`.
  - Keep the `Tofu` policy line.

- [ ] **Step 4: Rename the callers.** `palette`→`tokens` and `.muted`→`.text_low` everywhere listed under Files. Behaviour does not change yet.

- [ ] **Step 5: Run the whole crate.**
  Run: `cargo test -p fleet-gui`
  Expected: all pass, including the four new tests.

---

### Task 2: Theme choice in `GuiSettings`, saved without clobbering

**Files:**
- Modify: `crates/fleet-client/src/config.rs` (`GuiSettings`, from line 85).
- Modify: `crates/fleet-gui/src/theme.rs` (add `preference`).
- Modify: `crates/fleet-gui/src/shell.rs`:
  - `CloseFlow` saves through `GuiSettings::update` (the `settings_ui` "Ask me again" path and the `dialog_ui` remember path);
  - the startup paths in `Root::new`/`main.rs` apply the stored theme.
- Modify: `crates/fleet-gui/src/main.rs`. In the `run_native` closure, after `theme::apply`, call `cc.egui_ctx.set_theme(theme::preference(GuiSettings::load(&config::gui_settings_path(&home())).theme))`.
- Test: `crates/fleet-client/tests/config.rs`, `crates/fleet-gui/tests/shell.rs`.

**Interfaces:**
- Produces (fleet-client):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeChoice { #[default] System, Light, Dark }
impl ThemeChoice { pub fn next(self) -> ThemeChoice; /* System→Light→Dark→System */ pub fn label(self) -> &'static str; /* "System" | "Light" | "Dark" */ }
pub struct GuiSettings { #[serde(default)] pub close_choice: Option<CloseChoice>, #[serde(default)] pub theme: ThemeChoice }
impl GuiSettings { pub fn update(path: &Path, change: impl FnOnce(&mut GuiSettings)) -> std::io::Result<GuiSettings>; } // load fresh, change, save, return
```

- Produces (fleet-gui): `pub fn preference(choice: ThemeChoice) -> egui::ThemePreference` in `theme.rs`.

- [ ] **Step 1: Write the failing tests** in `crates/fleet-client/tests/config.rs`:
  - **`theme_choice_cycles_and_names_itself`:** `System.next() == Light`, `Light.next() == Dark`, `Dark.next() == System`, and `System.label() == "System"`.
  - **`a_settings_file_without_theme_loads_as_system`:** write `close_choice = "stop_daemon"\n` to the path. Then `GuiSettings::load` gives `theme == System` and `close_choice == Some(StopDaemon)`.
  - **`update_keeps_fields_it_does_not_touch`:**
    1. `update(p, |s| s.theme = Dark)`;
    2. `update(p, |s| s.close_choice = Some(KeepRunning))`;
    3. `load(p)` has both values.
  - Fix the existing `gui_settings_round_trip_and_create_the_directory` literal with `..Default::default()`.

- [ ] **Step 2: Write the failing test** in `crates/fleet-gui/tests/shell.rs`:
  - **`remembering_a_close_choice_keeps_the_saved_theme`:**
    1. Build a `CloseFlow` on a temp settings path.
    2. Call `GuiSettings::update(path, |s| s.theme = ThemeChoice::Dark)` (as the top bar will).
    3. Drive the dialog with "Remember my choice" and then "Keep the daemon running", reusing the file's existing dialog-driving helpers.
    4. Assert `GuiSettings::load(path)` has `theme == Dark` and `close_choice == Some(KeepRunning)`.

- [ ] **Step 3: Run them and see them fail.**
  Run: `cargo test -p fleet-client --test config && cargo test -p fleet-gui --test shell`
  Expected: compile errors, then the shell test fails on `theme == System`.

- [ ] **Step 4: Implement it.**
  - `ThemeChoice`, `GuiSettings::update` and `theme::preference` as in Interfaces.
  - `CloseFlow` writes only through `GuiSettings::update`. It still keeps its in-memory copy for drawing the footer line.
  - `main.rs` applies the stored theme before the first frame.

- [ ] **Step 5: Run both crates.**
  Run: `cargo test -p fleet-client && cargo test -p fleet-gui`
  Expected: all pass.

---

### Task 3: Primitives — buttons, badges, tabs, icons, dot

**Files:**
- Modify: `crates/fleet-gui/src/widgets/{button,badge,tabs,mod}.rs`.
- Create: `crates/fleet-gui/src/widgets/icon.rs`.
- Modify: `crates/fleet-gui/src/widgets/field.rs` (the badge call sites).
- Test: `crates/fleet-gui/tests/widgets.rs`.

**Interfaces:**
- Consumes: `theme::{tokens, tone_color, tone_tint, sans, Weight, size, radius}`.
- Produces:

```rust
pub enum ButtonVariant { Primary, Default, Ghost, Danger, QuietDanger }
pub fn button(ui: &mut Ui, label: &str, variant: ButtonVariant) -> Response;   // same signature as today
pub fn toggle(ui: &mut Ui, label: &str, pressed: bool) -> Response;             // unchanged behaviour
pub enum BadgeTone { Neutral, Attention }
pub fn badge(ui: &mut Ui, text: &str, tooltip: &str, tone: BadgeTone) -> Response;  // replaces badge + badge_accent
pub fn tabs<Id: Copy + PartialEq>(ui: &mut Ui, tabs: &[(Id, &str)], selected: Id) -> Option<Id>; // underline style, same contract
pub enum Icon { Close, Theme(ThemeChoice) }
pub fn icon_button(ui: &mut Ui, icon: Icon, label: &str) -> Response;  // painted glyph, `label` is the accessible name and the tooltip
pub fn dot(ui: &mut Ui, color: Color32, diameter: f32) -> Response;    // painted circle, allocates diameter×diameter
```

- [ ] **Step 1: Write the failing tests** in `tests/widgets.rs`:
  - **Update `every_button_variant_draws_its_label`:** cover all five variants, with labels `Primary`, `Plain`, `Ghost`, `Confirm`, `Kill`.
  - **Update `badge_shows_text_with_label_as_tooltip`:** use `badge(ui, "core", "Repos", BadgeTone::Neutral)`.
  - **`icon_button_is_a_button_named_by_its_label`:** `icon_button(ui, Icon::Close, "Close")`. Then `h.get_by_role_and_label(Role::Button, "Close").click()` reports `clicked()` once.
  - **`tabs_mark_the_selected_tab_and_report_a_click_once`:** stays unchanged and must still pass, including the `Toggled` assertions. The underline tabs must keep exposing selected state.

- [ ] **Step 2: Run them and see them fail.**
  Run: `cargo test -p fleet-gui --test widgets`
  Expected: compile errors (`Primary`, `Ghost`, `BadgeTone`, `icon_button`).

- [ ] **Step 3: Implement the primitives.**
  - **Buttons, all with `radius::CONTROL`, `size::BUTTON` and a min height of about 24:**
    - `Primary`: fill `fill`, text `on_fill`, Medium.
    - `Default`: fill `surface`, 1 px `border`.
    - `Ghost`: no fill and no stroke, text `text_low`; `raised` fill on hover.
    - `Danger`: fill `danger`, text `on_fill` in light and `tokens(false).surface` in dark, so the text stays white on red in both. Take it from tokens and do not write `Color32::WHITE`.
    - `QuietDanger`: no fill and no stroke, text `danger`.
  - **Badges:**
    - Neutral: fill `raised`, text `text`.
    - Attention: fill `tone_tint(Attention)`, text `tone_color(Attention)`, Medium.
    - Both: radius 3, padding about 6×1, `size::META`+0.5.
  - **Tabs:** text `size::BUTTON`, `text_high` when selected and `text_low` otherwise. The selected tab gets a 2 px `text_high` underline. Set `WidgetInfo::selected(WidgetType::Button, enabled, selected, label)` so `Toggled` still reports.
  - **Icons:**
    - `Icon::Close` is two painted diagonal lines.
    - `Icon::Theme` is a painted circle: outlined for System, filled `text_low` for Light, and half-filled for Dark. Any clear distinct glyph per state will do.
    - Both are 26×26 with a `border` outline. Set the label with `response.widget_info(...)` and add `on_hover_text(label)`.

- [ ] **Step 4: Run the crate.**
  Run: `cargo test -p fleet-gui`
  Expected: all pass.

---

### Task 4: Board, columns and cards

**Files:**
- Modify: `crates/fleet-gui/src/components/{board,column,card}.rs`, `crates/fleet-gui/src/widgets/field.rs`.
- Test: `crates/fleet-gui/tests/board.rs`.

**Interfaces:**
- Consumes: Task 3 primitives and Task 1 theme.
- Produces (signature changes; `app.rs` passes `self.preview_agent()`):

```rust
pub fn board(ui: &mut Ui, schema: &UiSchema, agents: &[Agent], now: i64, states: &mut HashMap<String, CardState>, selected: Option<&str>) -> Vec<Command>;
pub fn column(ui: &mut Ui, def: &ColumnDef, schema: &UiSchema, agents: &[&Agent], now: i64, states: &mut HashMap<String, CardState>, selected: Option<&str>) -> Vec<Command>;
pub fn card(ui: &mut Ui, schema: &UiSchema, agent: &Agent, now: i64, state: &mut CardState, selected: bool) -> Vec<Command>;
```

- **Card slots.** The card places each visible schema field by key and kind, inside `card.rs`, and draws the slots in this order:
  - Meta: `repos`, then any `Duration` field aligned right.
  - Title: `task`.
  - Badges: any other `Badge` field.
  - Activity: `activity`.
  - Branch: any `Mono` field.
  - Body: anything else, drawn by `field()` as today.
- In the Meta slot each repo is its own label, in mono `size::META` `text_low`. The duration's hover text is the field label.

- [ ] **Step 1: Update the harness and existing tests** in `tests/board.rs`:
  - `board(…, None)` in `harness`.
  - In `card_answer_returns_the_text_unchanged` and `empty_answer_is_disabled`, the button is `"Send"`.

- [ ] **Step 2: Write the failing tests:**
  - **`meta_row_puts_repos_left_and_time_right_on_one_line`:** one Working agent in `core`. Assert `y("core")` ≈ `y("5m")` within 2, `x("core") < x("5m")`, and `y("core") < y(task)`.
  - **`long_unbroken_task_and_branch_stay_inside_the_card`:** task = `"x".repeat(300)`, branch = `"fleet/" + "y".repeat(300)`, `with_size(1400, 800)`. Let `col_w = 1400/5`. Assert:
    - the task label's rect width ≤ `col_w`;
    - its height ≤ 2.5 × the body row height (two lines);
    - the branch label's height ≤ 1.5 × the meta row height;
    - `x("Review") - x("Needs you")` is unchanged from the same board with a short task.
  - **`input_action_reads_send_with_the_action_as_hint`:** for a Needs-you agent, `Role::Button "Send"` exists and `query_by_role_and_label(Role::Button, "Answer")` is none.
  - **`working_card_shows_a_dot_and_draws_no_continuous_repaint`:** one Working agent, then `h.run()`. If the dot caused continuous repaints, `run` would panic. Also assert `h.get_by_label("running Bash")` exists.

- [ ] **Step 3: Run them and see them fail.**
  Run: `cargo test -p fleet-gui --test board`
  Expected: the new tests fail, and the old ones fail on the signature.

- [ ] **Step 4: Implement.**
  - **Board:** `ui.columns` with `item_spacing.x = 0`. Paint 1 px `border` vertical lines between columns. No outer frames and no top bars.
  - **Column:**
    - The header is a `surface` strip with a bottom border, containing `dot(tone, 7)`, the title in Medium `text_high` 12.5, and the count in `text_low` 11.5.
    - The body scrolls on `bg` with padding 8 and 6 between cards.
    - The empty hint is `size::SMALL` `text_low`.
  - **Card:**
    - Frame fill `surface`, stroke `border` (`text_low` when `selected`), `radius::CARD`, inner margin about 8×9. No tone stripe.
    - Title: a `LayoutJob` with `wrap.max_rows = 2`, `break_anywhere = true` and `overflow_character = Some('…')`; hover shows the full text.
    - Branch: `Label::truncate()` with hover showing the full text.
    - Activity: draw `dot(active tone, 5)` first when `column_for(schema, agent).tone == Tone::Active`.
    - Actions: Pause/Resume are `Ghost`; Kill is `QuietDanger`, then a `Danger` "Confirm Kill".
    - The input action is a joined control: a TextEdit with hint `"{label}…"` and a `Primary` "Send" flush right. Enter still sends.

- [ ] **Step 5: Run the crate.**
  Run: `cargo test -p fleet-gui`
  Expected: `board.rs` passes. `app.rs` may still fail on the board call until Task 5; if so, pass `self.preview_agent()` in `app.rs` now.

---

### Task 5: Top bar, repo dropdown and theme switch

**Files:**
- Modify: `crates/fleet-gui/src/components/repo_filter.rs` (rewrite), `crates/fleet-gui/src/components/mod.rs` (export `filter_summary`), `crates/fleet-gui/src/app.rs` (`FleetApp::ui` header section, around line 240), `crates/fleet-gui/src/shell.rs` (`Root` passes the settings path to the app).
- Test: `crates/fleet-gui/tests/panels.rs` (repo filter section), `crates/fleet-gui/tests/app.rs`.

**Interfaces:**
- Consumes: `ThemeChoice` and `GuiSettings::update` (Task 2); `icon_button` and `dot` (Task 3).
- Produces:

```rust
// components/repo_filter.rs
pub fn filter_summary(selected: &[String]) -> String;   // [] → "All repos", [a] → "a", n>1 → "{n} repos"
pub fn repo_filter(ui: &mut Ui, repos: &[(String, usize)], selected: &mut Vec<String>) -> bool;  // (repo, agent count)
// app.rs
impl FleetApp { pub fn with_settings(self, path: PathBuf) -> FleetApp; pub fn theme_choice(&self) -> ThemeChoice; }
```

- **Repo dropdown:**
  - An `egui::ComboBox::from_id_salt("repo_filter")` whose `selected_text` is a small `text_low` "Repo" followed by `filter_summary`, with `close_behavior(PopupCloseBehavior::CloseOnClickOutside)`.
  - Inside the popup:
    - a checkbox row "All repos", checked when `selected` is empty; clicking it clears the selection;
    - a separator;
    - one checkbox row per repo, with the count right-aligned in mono `text_low`.
  - The order of the selection stays click order, as today.
- **Top bar:**
  - An `egui::Panel::top("top_bar")` with fill `surface` and a 1 px `border` bottom stroke. Margins about 14×10.
  - Contents: `fleet-board` in `TextStyle::Heading`, then the live dot and label (unchanged texts), the repo dropdown, a flexible space, `icon_button(Icon::Theme(choice), &format!("Theme: {}", choice.label()))`, and `button("New agent", Primary)`.
  - The New agent button only sets `self.spawn.open = true`; the dialog itself is Task 6.
  - When `self.problem` is `Some`, a second row in the same panel shows it in `danger` `size::SMALL`.
- **Theme click:**
  1. `choice = choice.next()`;
  2. `ctx.set_theme(theme::preference(choice))`;
  3. if `settings_path` is set, `GuiSettings::update(path, |s| s.theme = choice)`.
  - `with_settings` reads the initial choice from the file. `Root` calls it with its `settings_path` when it builds the app.

- [ ] **Step 1: Rewrite the repo filter tests** in `tests/panels.rs`:
  - **`filter_summary_names_the_selection`:** `[]` gives "All repos", `["core"]` gives "core", and `["core","portal"]` gives "2 repos".
  - **`repo_filter_toggles_repos_in_click_order`:**
    1. Open with `h.get_by_role(Role::ComboBox).click(); h.run();`.
    2. Click `get_by_role_and_label(Role::CheckBox, "core")`, then "portal", then "core", then "portal". Assert the selection after each click: `["core"]`, then `["core","portal"]`, then `["portal"]`, then empty.
    3. The popup stays open between clicks.
  - **`all_repos_clears_the_selection`:** start with `["core"]`, open, click `CheckBox "All repos"`; the selection is empty and the function returned `true` once.
  - **`repo_filter_reports_a_change_once_per_click`:** keep it, opening the dropdown first.

- [ ] **Step 2: Update and add app tests** in `tests/app.rs`:
  - **`repo_filter_shows_agents_touching_any_selected_repo`:** open `Role::ComboBox` and click `CheckBox "portal"`. Keep the rest of the assertions.
  - **`theme_switch_cycles_and_saves_the_choice`:**
    1. `app().with_settings(tmp.join("gui.toml"))`.
    2. Click `Button "Theme: System"`, then `h.run()`.
    3. Assert `h.state().theme_choice() == Light`, `ctx.options(|o| o.theme_preference) == ThemePreference::Light`, `GuiSettings::load(path).theme == Light`, and that `Button "Theme: Light"` exists.
  - **`action_error_shows_under_the_top_bar`:** `handle_result(AppResult::Action(Err("agent is not running".into())))`. The label exists, and its `y` is greater than `y("fleet-board")` and less than `y("Queued")`.

- [ ] **Step 3: Run them and see them fail.**
  Run: `cargo test -p fleet-gui --test panels --test app`
  Expected: failures on the new tests and the signature.

- [ ] **Step 4: Implement** `repo_filter`, `filter_summary`, the top bar, `with_settings`, `theme_choice` and the theme click, as described above. Counts come from `self.snapshot.agents`: the number of agents with a workspace in that repo. The repo list is built as today, from configured repos plus repos seen on agents.

- [ ] **Step 5: Run the crate.**
  Run: `cargo test -p fleet-gui`
  Expected: all pass, except the spawn tests, which Task 6 rewrites. To keep the tree green, Task 5 may keep drawing the old `spawn_form` under the top bar until Task 6 removes it.

---

### Task 6: New agent dialog

**Files:**
- Delete: `crates/fleet-gui/src/components/spawn_form.rs`.
- Create: `crates/fleet-gui/src/components/new_agent.rs`.
- Modify: `crates/fleet-gui/src/components/mod.rs` (exports).
- Modify: `crates/fleet-gui/src/app.rs` (`handle_result` Spawn arms, line ~165; drawing).
- Test: `crates/fleet-gui/tests/panels.rs` (spawn section), `crates/fleet-gui/tests/app.rs`.

**Interfaces:**
- Produces:

```rust
#[derive(Default)]
pub struct SpawnState { pub open: bool, pub repo: String, pub prompt: String, pub busy: bool, pub error: Option<String> }
/// Draws nothing unless `st.open`. Returns (repo, prompt) when the user starts the agent.
pub fn new_agent_dialog(ctx: &egui::Context, repos: &[String], st: &mut SpawnState) -> Option<(String, String)>;
```

- **Layout:**
  - `egui::Modal` with a frame of `surface`, `radius::DIALOG` and width about 440.
  - Header: "New agent" in `TextStyle::Heading` and `icon_button(Icon::Close, "Close")`.
  - Body: a "Repo" caption over a `ComboBox::from_id_salt("new_agent_repo")` of `repos`, then a "Prompt" caption over a multiline `TextEdit` (`desired_rows` 4), labelled by the caption so `Role::MultilineTextInput` has a name.
  - The prompt gets focus on the frame the dialog opens.
  - Footer: a "Ctrl+Enter" hint in mono `text_low`, then `button("Cancel", Default)` and `button("Start agent", Primary)`, which is disabled unless the spawn is ready (as today).
  - `st.error` shows in `danger` above the footer.
- **Keys and closing:**
  - Consume `Modifiers::COMMAND + Key::Enter` with `ui.input_mut(|i| i.consume_key(...))` before the TextEdit is drawn, so no newline is inserted.
  - Escape, ✕, Cancel and `modal.should_close()` all set `st.open = false` and keep `prompt`.
- **Results in `app.rs`:**
  - `Spawn(Ok)`: `busy=false`, `open=false`, `prompt.clear()`, `error=None`.
  - `Spawn(Err(e))`: `busy=false` and `error=Some(e.clone())`. If `!open`, also set `self.problem = Some(format!("Could not start the agent: {e}"))`.

- [ ] **Step 1: Rewrite the spawn tests** in `tests/panels.rs`:
  - The harness builds `SpawnState { open: true, .. }` and calls `new_agent_dialog(ui.ctx(), &repos(), &mut s.st)` with `with_max_steps(60)`.
  - Keep `spawn_is_disabled_until_the_prompt_has_text`, `spawn_returns_repo_and_prompt_unchanged`, `spawn_uses_the_chosen_repo`, `spawn_error_is_shown_and_the_prompt_kept` and `a_busy_spawn_cannot_be_sent_twice` with their assertions unchanged.
  - **`ctrl_enter_starts_without_adding_a_newline`:**
    1. Type `"do it"`.
    2. `h.key_press_modifiers(Modifiers::COMMAND, Key::Enter); h.run();`
    3. `started == [("core","do it")]`.
  - **`plain_enter_inserts_a_newline`:** type "a", press `Key::Enter`, type "b". The prompt is `"a\nb"` and nothing started.
  - **`escape_and_cancel_close_and_keep_the_text`:**
    1. Type "keep me" and press Escape. Then `!st.open` and `prompt == "keep me"`.
    2. Reopen with `st.open = true`, click "Cancel", and check the same.
  - **`closed_dialog_draws_nothing`:** with `open: false`, `query_by_label("New agent")` is none.

- [ ] **Step 2: Update and add app tests** in `tests/app.rs`:
  - **`spawn_success_clears_the_prompt_and_failure_keeps_it_with_the_error`:** click `Button "New agent"` first, then run as before. On `Ok`, also assert `!spawn_state().open`.
  - **`spawn_failure_after_closing_the_dialog_shows_in_the_problem_line`:**
    1. Open, type, start, press Escape.
    2. `handle_result(Spawn(Err("boom")))`.
    3. `h.state().problem() == Some("Could not start the agent: boom")`, and `spawn_state().open` is still false.
  - **`spawn_success_after_closing_does_not_reopen`:** do the same with `Ok`. `open` is false and the prompt is empty.

- [ ] **Step 3: Run them and see them fail.**
  Run: `cargo test -p fleet-gui --test panels --test app`
  Expected: compile errors, then failures.

- [ ] **Step 4: Implement** `new_agent.rs`, the `app.rs` result arms, and the drawing call. Draw the dialog after the panels, at the end of `FleetApp::ui`. Remove `spawn_form` and its export.

- [ ] **Step 5: Run the crate.**
  Run: `cargo test -p fleet-gui`
  Expected: all pass.

---

### Task 7: Side panel

**Files:**
- Modify: `crates/fleet-gui/src/components/preview.rs`, `crates/fleet-gui/src/app.rs` (`open_preview` ~line 190; the `Panel::right` call ~line 290).
- Test: `crates/fleet-gui/tests/panels.rs` (preview section), `crates/fleet-gui/tests/app.rs`.

**Interfaces:**
- Produces: `PreviewState` gains `pub path: String`, which `open_preview` sets to `"{repos} / {branch}"`. Repos are joined with `", "` from `field_value(agent, "repos")`, and the branch comes from `field_value(agent, "branch")`. Everything else is unchanged.

- **Panel:**
  - `Panel::right("preview")`: resizable, `default_size(420.0)`, frame fill `surface` with a 1 px `border` on the left.
  - **Header row:** `path` in mono 11.5 `text_high`, truncated with hover, and `icon_button(Icon::Close, "Close")` on the right. A bottom border follows.
  - **Under the header:** `title` in Medium 13 `text_high`, wrapping.
  - **Tab row:** `tabs(…)` (underline) on the left. When the Diff tab is selected, `button("Refresh", Ghost)` sits on the right. A bottom border follows.
  - **Body:** a frame with fill `bg` and padding 10×12. The rows view is unchanged except for colours:
    - `+` lines: `tone_color(Done)`;
    - `-` lines: `danger`;
    - `@@` lines: `tone_color(Active)`;
    - `+++`/`---` lines: `text_low`;
    - repo headings: Medium `text_high`;
    - notes, "Session ended" and "Loading…": `size::SMALL` `text_low`.

- [ ] **Step 1: Write the failing tests:**
  - **`header_shows_repo_and_branch_path`** (in `panels.rs`): `PreviewState { path: "core / fleet/x".into(), .. }`. `get_by_label("core / fleet/x")` exists, and `Role::Button "Close"` returns `PreviewAction::Close`, which replaces the old `close_button_returns_close` click.
  - **`opening_a_card_fills_the_panel_path`** (in `app.rs`): an agent in `core` with branch `fleet/x`, "open me" clicked. `get_by_label("core / fleet/x")` exists.

- [ ] **Step 2: Run them and see them fail.**
  Run: `cargo test -p fleet-gui --test panels --test app`
  Expected: compile error on `path`, then failures.

- [ ] **Step 3: Implement** the panel as described.

- [ ] **Step 4: Run the crate.**
  Run: `cargo test -p fleet-gui`
  Expected: all pass. That includes the existing tests for preview text, diff, the 1 MiB file and long lines.

---

### Task 8: Shell screens, close dialog and footer

**Files:**
- Modify: `crates/fleet-gui/src/shell.rs` (`settings_ui` ~line 91, `dialog_ui`, `Root::ui` startup/failed stages ~line 220–260), `crates/fleet-gui/src/components/close_dialog.rs`.
- Test: `crates/fleet-gui/tests/shell.rs`.

- **Footer:** `settings_ui` draws an `egui::Panel::bottom("close_choice")` with fill `surface` and a top `border`. It holds the existing sentence in `size::SMALL` `text_low` and `button("Ask me again", Ghost)`. `Root::ui` already calls it before `app.ui`, so the bottom panel is laid out before the central board.
- **Modals** ("Stopping the daemon…", "The daemon did not stop", the close dialog):
  - frame `surface`, `radius::DIALOG`, inner margin 16;
  - headings in `TextStyle::Heading`;
  - body text in `text`.
- **Close dialog:**
  - "Keep the daemon running" is `Primary` and keeps the focus;
  - "Stop the daemon too" is `QuietDanger`;
  - "Cancel" is `Default`;
  - "Remember my choice" is a stock checkbox, styled by the visuals;
  - the hint line is `size::SMALL` `text_low`.
- **"The daemon did not stop":** "Close anyway" is `Default` and "Cancel" is `Primary`.
- **Startup stages** (`Starting`, `Failed`): a centred `surface` card, `radius::DIALOG`, max width 520.
  - Starting shows "Starting the daemon…" in body `text_low`.
  - Failed shows the heading, the message in mono, the state directory in `size::SMALL` `text_low`, and `button("Retry", Primary)`.
  - Labels are unchanged.

- [ ] **Step 1: Write the failing test:**
  - **`remembered_choice_line_sits_at_the_bottom`:**
    1. A `Root::running` with `close_choice = Some(KeepRunning)` saved.
    2. `y("Ask me again") > y("Queued")` and `y("Ask me again") > 700.0` in an 800-high harness.

- [ ] **Step 2: Run it and see it fail.**
  Run: `cargo test -p fleet-gui --test shell`
  Expected: the new test fails, because today the line sits above the board.

- [ ] **Step 3: Implement** the footer and the restyle above. All existing `shell.rs` tests must pass unchanged, including the focus-on-safe-choice test.

- [ ] **Step 4: Run the workspace checks.**
  Run: `cargo test -p fleet-gui && cargo clippy -p fleet-gui -p fleet-client -- -D warnings`
  Expected: all pass, with no warnings.

- [ ] **Step 5: Grep for leftovers.**
  Run: `grep -rnE 'Color32::(from_rgb|WHITE|BLACK)|#[0-9a-f]{6}' crates/fleet-gui/src | grep -v 'src/theme.rs'`
  Expected: no output.

---

### Task 9: Docs and acceptance on a real window

**Files:**
- Modify: `docs/ui.md` (add an "egui client" section).
- Create: `docs/superpowers/findings/egui-restyle.md`, `docs/superpowers/findings/egui-restyle/*.png`.

- [ ] **Step 1: Write the egui section of `docs/ui.md`** (about six lines):
  - `theme.rs` is the only place for colours, sizes, radii and fonts;
  - `widgets/` is the only place buttons, badges, tabs, icons and dots are drawn;
  - components never write a colour;
  - the fonts are vendored under `crates/fleet-gui/assets/fonts/` with their OFL licence;
  - the window must stay idle (no animation).

- [ ] **Step 2: Build and check the cross-compile guard.**
  Run: `cargo build --release -p fleet-gui && cargo check -p fleet-gui --target x86_64-pc-windows-gnu`
  Expected: both succeed. If the Windows target is not installed, use the guard command from `build.sh`. Note the binary size against the previous `dist/` binary in the findings; expect it to grow by about 560 KB.

- [ ] **Step 3: Capture the real window on Xvfb**, as in `docs/superpowers/findings/egui-spike.md`:
  1. Start `Xvfb :99 -screen 0 1400x800x24 +extension GLX` and run `DISPLAY=:99 fleet-gui`.
  2. Get the window id from `xwininfo -root -tree` and capture with `import -window <id> <file>.png`.
  3. Drive agents through the daemon API (no `xdotool`), as in `egui-acceptance.md`.
  4. Get the theme by writing `theme = "dark"` or `"light"` to `~/.config/fleet-board/gui.toml` before launch.
  5. Capture in both themes:
     - `board-dark.png` and `board-light.png`: one agent in Needs you, one in Review with changes;
     - `panel-dark.png` and `panel-light.png`: the same board with the side panel open. Open it by temporarily setting `preview` through a debug run, or skip this capture and note it as covered by the UI tests;
     - `new-agent-*.png` and `repo-menu-*.png`: if they cannot be opened without clicks, note them as covered by the UI tests.

- [ ] **Step 4: Write `egui-restyle.md`:**
  - what was captured;
  - a side-by-side judgement against the mockup, listing every visible difference;
  - confirmation that no default egui widget remains;
  - the binary size change.

- [ ] **Step 5: Final run.**
  Run: `cargo test --workspace`
  Expected: all pass.

---

### Task 10: Focus-gated animation

Added 2026-10-23 at Antonio's request. Spec section "Animation".

**Files:**
- Modify: `crates/fleet-gui/src/theme.rs` (animation gate and pulse helper).
- Modify: `crates/fleet-gui/src/app.rs` (`FleetApp::ui`: set `animation_time` from the gate each frame).
- Modify: `crates/fleet-gui/src/components/card.rs` (the Working dot uses the pulse).
- Test: `crates/fleet-gui/tests/widgets.rs`, `crates/fleet-gui/tests/board.rs`.

**Interfaces:**
- Produces, in `theme.rs`:

```rust
pub const ANIMATION_FRAME: std::time::Duration = std::time::Duration::from_millis(100);
pub const ANIMATION_TIME: f32 = 0.12;   // egui transition length while animating
/// True only while the window has focus; every animation in the app asks this, nowhere else reads focus.
pub fn animate(ctx: &egui::Context) -> bool;          // ctx.input(|i| i.viewport().focused) == Some(true)
/// Opacity 0.35..=1.0 breathing with `period`; when !animate returns 1.0 and requests nothing,
/// otherwise schedules `request_repaint_after(ANIMATION_FRAME)`.
pub fn pulse(ui: &Ui, period: f32) -> f32;
```

- Consumes: `dot(ui, color, diameter)` from Task 3. The card calls it with `tone_color(Active).gamma_multiply(theme::pulse(ui, 1.6))`.

- [ ] **Step 1: Write the failing tests:**
  - **`animate_is_off_without_focus`** (in `widgets.rs`): in `Harness::new_ui`, `theme::animate(ui.ctx())` is false, and `theme::pulse(ui, 1.6) == 1.0`.
  - **`animate_and_pulse_follow_focus`** (in `widgets.rs`):
    1. Set the root viewport's `focused = Some(true)` through the harness's raw input (`RawInput.viewports`).
    2. Advance with `h.step()`, not `h.run()`, since a scheduled repaint would make `run` loop.
    3. Assert `animate` is true, `pulse` is in `0.35..=1.0`, and the pulse value differs between two steps about 0.4 s apart. Advance time through `RawInput.time` or `predicted_dt`.
    4. Set `focused = Some(false)`, step, and assert `pulse == 1.0`.
  - **`an_unfocused_board_with_a_working_card_requests_no_repaint`** (in `board.rs`): one Working agent, and `h.run()` completes. This is the existing guard; keep it.
  - **`egui_transitions_follow_the_gate`** (in `app.rs` tests): after one frame, `ctx.style().animation_time == 0.0` while unfocused, and `== ANIMATION_TIME` after focusing and stepping.

- [ ] **Step 2: Run them and see them fail.**
  Run: `cargo test -p fleet-gui --test widgets --test board --test app`
  Expected: compile errors (`animate`, `pulse`, `ANIMATION_TIME`).

- [ ] **Step 3: Implement** `animate`, `pulse`, the card dot and the per-frame `animation_time`. `pulse` uses `ui.input(|i| i.time)`. The breathing curve is `0.675 + 0.325 * cos(2π t / period)`. Nothing else in the crate reads `viewport().focused`.

- [ ] **Step 4: Run the crate.**
  Run: `cargo test -p fleet-gui` and the clippy gate.
  Expected: all pass.

---

### Task 11: Answer from the side panel

Added 2026-10-24 at Antonio's request. Spec: Card "Actions" and Side panel "Reply box".

**Files:**
- Create: `crates/fleet-gui/src/components/reply.rs`.
- Modify: `crates/fleet-gui/src/components/{card,preview,mod}.rs`, `crates/fleet-gui/src/app.rs`.
- Test: `crates/fleet-gui/tests/{board,panels,app}.rs`.

**Interfaces:**
- Produces:

```rust
// components/reply.rs
/// The reply box at the bottom of the side panel. Returns the text to send on Ctrl+Enter or a Send click
/// (the caller clears `text`). Enter inserts a newline. `focus` asks for the cursor this frame.
pub fn reply_box(ui: &mut Ui, label: &str, text: &mut String, focus: bool) -> Option<String>;
// components/preview.rs
pub struct Reply<'a> { pub label: &'a str, pub text: &'a mut String }
pub enum PreviewAction { None, LoadDiff, Close, Reply(String) }      // Reply added
pub fn preview_pane(ui: &mut Ui, st: &mut PreviewState, reply: Option<Reply<'_>>) -> PreviewAction;
// PreviewState gains `pub focus_reply: bool` (set by open_preview, cleared after the first frame that draws the box)
// app.rs: FleetApp gains `replies: HashMap<String /*agent id*/, String>`
impl FleetApp { pub fn reply_text(&self, agent_id: &str) -> &str; }   // for tests
```

- **Card:** `card.rs` no longer draws input actions. `visible_actions` entries with `input.is_some()` are skipped. Remove `CardState.typed` if nothing else uses it.
- **App:**
  - Before drawing the panel, find the previewed agent's first visible action with `input.is_some()` (`fleet_client::schema::visible_actions`). If there is one, pass `Some(Reply { label: &action.label, text: replies.entry(id) })`.
  - `PreviewAction::Reply(text)` becomes `Command::Run { action, agent, input: Some(text) }` through the existing `run_command`, and the stored reply for that agent is cleared.

- [ ] **Step 1: Move and write the failing tests.**
  - **`board.rs`:**
    - Replace `card_answer_returns_the_text_unchanged`, `empty_answer_is_disabled` and `two_cards_keep_separate_answer_boxes` with **`needs_you_card_has_no_answer_field`**. A Needs-you card has no `Role::TextInput`, no `Role::MultilineTextInput` and no `Button "Send"`, but still has the blocked badge and "Kill".
    - Delete any answer-specific seam test.
  - **`panels.rs` (reply box):**
    - **`reply_box_sends_on_ctrl_enter_unchanged`:** type `"$(x) \"q\" {a} é -n"`, then `key_press_modifiers(COMMAND, Enter)`. The result is `Some(that text)`, with no trailing newline.
    - **`reply_box_enter_inserts_a_newline`:** type "a", Enter, "b". The text is `"a\nb"` and nothing is sent.
    - **`reply_box_send_button_sends_and_is_disabled_when_blank`:** clicking Send on `"   "` sends nothing; on "1" it returns `Some("1")`.
    - **`preview_without_reply_has_no_reply_box`:** `preview_pane(…, None)` has no `MultilineTextInput` and no Send.
  - **`app.rs`:**
    - **`opening_a_needs_you_card_focuses_the_reply_box`:** click the card. `Role::MultilineTextInput` exists and is focused.
    - **`a_reply_is_kept_per_agent_and_cleared_when_sent`:**
      1. Two Needs-you agents.
      2. Open A, type "draft a".
      3. Open B, type "draft b".
      4. Reopen A: the field shows "draft a".
      5. Ctrl+Enter: `reply_text("a") == ""` and `reply_text("b") == "draft b"`.
    - **`a_working_agent_has_no_reply_box`:** open a Working agent; there is no `MultilineTextInput`.

- [ ] **Step 2: Run them and see them fail.**
  Run: `cargo test -p fleet-gui --test board --test panels --test app`
  Expected: compile errors, then failures.

- [ ] **Step 3: Implement.**
  - **Reply box:** pinned to the panel bottom with a nested bottom panel (`egui::Panel::bottom` inside the side panel's `Ui`). Fill `surface`, a top `border`, padding 12.
  - **Field:** a multiline `TextEdit` about 3 rows high, full width, hint `"{label}…"`.
  - **Footer row:** a mono `size::SMALL` `text_low` "Ctrl+Enter" hint on the left and `button("Send", Primary)` on the right, disabled when the text is blank.
  - **Ctrl+Enter:** consume `Modifiers::COMMAND + Key::Enter` before the TextEdit draws, as `new_agent.rs` does.
  - **Focus:** focus the field when `focus` is true.

- [ ] **Step 4: Run the crate.**
  Run: `cargo test -p fleet-gui` and the clippy gate.
  Expected: all pass.
