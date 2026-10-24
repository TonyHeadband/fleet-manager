# egui client restyle — design

Date: 2026-10-23. Scope: `crates/fleet-gui` only. The approved mockup is `2026-10-23-egui-restyle-mockup.html` next to this file (open it in a browser; the Dark/Light switch flips the mockup). It is also published at https://claude.ai/artifact/8FXeJZz6QdmrBoXhyvya4b. Where this document and the mockup disagree, this document wins.

## Why

The v0.1 window works but looks like a prototype. The screenshots in `docs/superpowers/findings/egui-acceptance/` show why:

- The header is crammed: the title is clipped and the spawn form spills onto its own line.
- Each column is an outlined box with a coloured bar on top, so the board reads as a wireframe.
- Card text jumps between `.small()` and the normal size with nothing in between, and duration takes a row of its own.
- The buttons, combo box and text edits are egui's defaults.
- Background, column and card are almost the same dark.

Antonio wants it to look like a finished, professional product, with a polished dark theme and a polished light theme. vibe-kanban (BloopAI) is the reference for the look.

## Goals

- A dark and a light theme, both deliberate, from one token set.
- vibe-kanban's visual language:
  - neutral greys with no blue tint;
  - IBM Plex Sans for text and IBM Plex Mono for code-like text;
  - small corner radii;
  - thin dividers instead of boxes;
  - a coloured dot before each column title.
- Layout B from the mockup:
  - one slim top bar;
  - the board takes all the height below it;
  - starting an agent moves into a "New agent" dialog;
  - the repo filter becomes a dropdown.
- A theme switch in the top bar, remembered between launches.
- Every existing behaviour keeps working: answer, pause/resume, kill with confirmation, preview with Terminal/Diff, reconnecting, close flow, startup screen.

## Non-goals

- The web UI. It stays frozen at v1 (see `TODO.md`, "Bring the web UI back to parity"). The egui palette no longer has to equal `web/src/styles.css`.
- New features: search, column "+" buttons, drag and drop, avatars, sidebar, issue IDs. Only what the window already does, restyled.
- Changes to daemon behaviour or to the HTTP API.

## Anti-goals (a failure even if it works)

- **A half-restyle.** If one screen still shows egui's default grey pill buttons or default combo box next to the new look, the restyle has failed. That covers the startup screen, the close dialog and the "daemon did not stop" dialog.
- **Fonts fetched at runtime.** The binary must run air-gapped, so fonts are compiled in.
- **Agent output that stops rendering a glyph.** The `Tofu` missing-glyph policy and egui's default fonts stay as fallbacks behind Plex.

## Constraints

- egui/eframe 0.37 with the glow renderer, as today. No new UI crates. Fonts go in with `include_bytes!`.
- **egui does not do font weights.**
  - `RichText::strong()` only changes colour.
  - Medium and semi-bold text therefore need their own font files registered as named families.
  - Vendor static TTFs: IBM Plex Sans Regular and Medium (SemiBold if a heading needs it), and IBM Plex Mono Regular.
  - Put them, with the SIL OFL licence text, under `crates/fleet-gui/assets/fonts/`.
  - Get them from IBM's `plex` GitHub release, then commit them to the repo so the build needs no network.
  - Expected binary growth is about 600–800 KB.
- UI tests (`egui_kittest`) find widgets by role and label. Keep visible labels unchanged unless this document changes them, so tests keep finding them.
- The 15 s repaint, the virtualised preview rows and the 2000-char line clip stay as they are.

## Tokens

Defined in `crates/fleet-gui/src/theme.rs`, which replaces today's `Palette`. Nothing outside `theme.rs` may write a literal colour.

| Token | Dark | Light | Use |
|---|---|---|---|
| `bg` | `#1c1c1c` | `#f2f2f2` | board background, input fields |
| `surface` | `#212121` | `#ffffff` | top bar, column headers, cards, side panel, dialogs, menus |
| `raised` | `#292929` | `#e3e3e3` | hover, selected menu row, neutral badge fill |
| `border` | `#333333` | `#d9d9d9` | every divider and outline |
| `text_high` | `#f5f5f5` | `#0d0d0d` | titles, task text, selected values |
| `text` | `#c4c4c4` | `#333333` | body |
| `text_low` | `#8f8f8f` | `#636363` | meta, hints, activity, branch, counts |
| `fill` / `on_fill` | `#f5f5f5` / `#141414` | `#141414` / `#ffffff` | primary button (New agent, Start agent, Send) |
| `danger` | `#e5534b` | `#c4302b` | Kill, errors, diff deletions |

- **Tones:** keep today's `tone_color` values (neutral `#8a94a6`; active `#6b9bff`/`#2f6fed`; attention `#f5a524`/`#d97706`; done `#4ade80`/`#15803d`; muted `#b39ddb`/`#7c5cc4`).
- **Attention badge:** the tinted background is the attention tone at about 14% alpha in dark and 12% in light.
- **Primary button colour:** plain black or white, not vibe-kanban's orange. *My call:* orange would compete with the amber "Needs you" tone.
- **Radii:** 3 px for buttons, inputs and badges; 4 px for cards; 6 px for dialogs and menus.

Type scale, set as egui `TextStyle`s so widgets pick it up:

| Style | Size | Family |
|---|---|---|
| Heading (app name, dialog title) | 15 | Plex Sans Medium/SemiBold |
| Body | 13 | Plex Sans |
| Card title | 12.5–13 | Plex Sans, `text_high` |
| Button | 12 | Plex Sans (Medium on primary) |
| Small (activity, hints, counts) | 11.5 | Plex Sans |
| Meta and branch | 10.5–11 | Plex Mono |
| Monospace (terminal, diff) | 11.5–12 | Plex Mono |

The mockup is CSS pixels. egui points at `pixels_per_point = 1` render about the same, so the builder can nudge sizes by ±0.5 for legibility.

Set `egui::Visuals` from the tokens:
- widget fills and strokes for every state;
- selection colour;
- `window_fill`, `panel_fill`, `extreme_bg_color`;
- popup and window shadows (soft and small);
- corner radii.

That way stock widgets (ComboBox popup, TextEdit, Checkbox, Modal, ScrollArea) match without per-call styling.

## Layout and components

**Top bar** (one row, `surface`, bottom border):
- Left to right:
  - `fleet-board` in the heading style;
  - the live dot with "live" or "reconnecting…" in small `text_low` (labels unchanged; tests look for them);
  - the repo dropdown;
  - a flexible gap;
  - the theme switch;
  - the "New agent" primary button.
- The app-level error (`self.problem`) shows as a thin danger line under the bar, not inside it.

**Repo dropdown** (replaces the toggle pills):
- The closed button reads `Repo` (small, `text_low`), then the current value, then a caret.
- The value is "All repos" when none is ticked, the repo name when one is ticked, and "N repos" when several are ticked.
- The open menu has an "All repos" row (clears the selection), a divider, then one row per repo. Each repo row has a checkbox, the name, and on the right its agent count in mono `text_low`.
- Multi-select behaviour is unchanged. The repo list is still configured repos plus any repo an agent uses.
- "Repo", not "Project": in fleet-board a project is one agent's unit of work.
- Tests that clicked the `portal` button now open the dropdown, then click the `portal` row.

**New agent dialog:**
- `egui::Modal`, width about 440 with a 6 px radius.
- The header has the title "New agent" and a close ✕.
- The body has two labelled fields, each label in small medium `text_low` above its field:
  - Repo: a dropdown of configured repos only;
  - Prompt: a multiline field, focused when the dialog opens.
- The footer has a "Ctrl+Enter" hint on the left, then Cancel and the primary "Start agent".
- Ctrl+Enter starts the agent. Escape, ✕, Cancel and a click outside all close the dialog and keep the typed text for next time.
- While the spawn is in flight, "Start agent" is disabled. On success the dialog closes and the prompt clears. On failure the dialog stays open with the error in `danger` above the footer, and the text is kept. If the user closed the dialog while the spawn was in flight, the failure also shows in the app-level danger line under the top bar, so it is never lost; the error stays in the dialog for when it is reopened. *My call.*
- Keep the labels "Prompt" and "Start agent" for the tests; tests now click "New agent" first.

**Board:**
- Five columns of equal width with 1 px `border` dividers between them. No outer boxes and no coloured top bars.
- Each column header sits on `surface` with a bottom border. It shows a 7 px tone dot, the title in medium `text_high`, and the count in `text_low` with tabular figures.
- The column body is `bg` and scrolls on its own. Cards are 6 px apart with 8 px padding.
- The empty hint is small `text_low`.

**Card:**
- `surface` fill, 1 px `border`, 4 px radius, padding about 8×9.
- The selected card (open in the side panel) gets a `text_low` border.
- No tone stripe on the card edge.
- Top to bottom:
  1. Meta row, mono `text_low`: repos on the left; duration (`status_since`) on the right.
  2. Task: `text_high`, at most two lines with an ellipsis. Full text on hover.
  3. Badges:
     - the blocked reason uses the attention tint;
     - Has changes, Paused and Ended use the neutral fill (`raised`) with `text`.
  4. Activity: small `text_low`. For Working, a small `active` dot comes first. It pulses only while animation is allowed (see **Animation**). Otherwise it is drawn fully on.
  5. Branch: mono `text_low`, one line with an ellipsis. Full branch on hover.
  6. Actions:
     - No answer field on the card. Answering happens in the side panel's reply box (below). A Needs-you card keeps its blocked badge and its other actions.
     - Pause and Resume are ghost buttons.
     - Kill is quiet danger text, which turns into a filled "Confirm Kill" as today.
- Clicking the card body still opens the side panel, and labels still must not swallow that click.
- **Field placement:** the schema already decides which fields exist and when they show. *My call:* the card groups them into the slots above by field key and kind inside `fleet-gui` (repos and duration into meta, `task` into title, badges into badges, and so on), the way `field.rs` already special-cases `task`, `activity` and `blocked_reason`. A field it does not know keeps today's rendering by kind, in the body. The `fleet-api` schema is not changed.
- **Answer button rename:** the button changes from "Answer" to "Send"; the field's hint text stays "Answer…". The tests that click it must be updated. (Since Task 11 the button and field live in the side panel's reply box.)

**Side panel** (today's preview):
- Resizable on the right with a default width of about 380–460, on `surface` with a left border.
- **Header row:** `repo / branch` in mono `text_high` with an ellipsis, then a ✕ close button. Its accessible label stays "Close" for the tests.
- **Under the header:** the task title in medium `text_high`, wrapping.
- **Tabs:** "Terminal" and "Diff" as text tabs on an underline. The selected tab is `text_high` with a 2 px underline; the other is `text_low`. "Refresh" (Diff only) is a ghost button aligned right on the tab row.
- **Body:** on `bg` in Plex Mono.
- **Diff colours:** additions use the done tone, deletions `danger`, hunks the active tone, and file headers `text_low`. A repo heading is medium `text_high`.
- **Reply box:** added 2026-10-24 at Antonio's request. It replaces the card's answer line, so you read the terminal and answer in one place.
  - It is pinned to the bottom of the panel, under Terminal/Diff, on `surface` with a top `border`. It shows only when the agent has a visible input action in the schema (today: Answer on Needs you). Nothing names a status.
  - It holds a multiline field with hint `"{action label}…"` (e.g. "Answer…"), about 3 rows. Below it sit a mono `text_low` "Ctrl+Enter" hint and a primary "Send" button, disabled while the text is blank.
  - **Enter inserts a new line; Ctrl+Enter sends** (Antonio's choice, the same as the New agent dialog).
  - Opening a card that has a reply box puts the cursor in it.
  - The text is kept per agent, so switching cards keeps a half-written answer. It clears when sent.
- "Session ended" and "Loading…" are small `text_low`.

**Footer status line:**
- The "When you close the window: … / Ask me again" strip from `shell.rs`.
- A slim bar on `surface` with a top border. Small `text_low` text, with "Ask me again" as a ghost button.

**Shell screens and dialogs:**
- "Starting the daemon…", "could not start its daemon", "Stopping the daemon…", "The daemon did not stop", the close dialog and "Cannot load the board".
- Same Modal styling as New agent, headings in the heading style, and buttons from the shared primitives.
- The safe default keeps the focus, as today.
- In the close dialog, "Keep the daemon running" becomes the primary button and "Stop the daemon too" stays quiet danger.

**Primitives** (`widgets/`), still the only place a button or badge is drawn:
- Button variants: `Primary`, `Default` (outlined), `Ghost`, `Danger` (filled), `QuietDanger`.
- `badge(tone)`: neutral or attention.
- `tabs`: the underline style.
- Add a dropdown/menu helper, plus a `dot` painter for the live indicator and column dots, which are painted rather than drawn as glyphs.

## Animation

Added 2026-10-23 at Antonio's request. egui redraws the whole window for every frame, so anything that moves costs a full frame each time.

- Every animation goes through one gate: `theme::animate(ctx) -> bool`. It is true only while the window has focus.
  - **Focused:** continuous motion is allowed, throttled to about 10 fps (`request_repaint_after(100 ms)`), and only while something animated is on screen.
  - **Unfocused or minimised:** nothing animates. The window redraws only on events and on the 15 s refresh, as before, and animated elements draw their resting state (for example the dot fully on).
  - egui's own transitions (`style.animation_time`) follow the same gate. They are 0 while unfocused.
- The first animation is the Working dot's pulse, a slow breathing of opacity with a period of about 1.6 s.
- Tests never report focus, so animation is off there and `Harness::run()` never sees a repaint loop.

## Theme switch

- An icon-sized button in the top bar cycles System → Light → Dark. Its tooltip names the current mode ("Theme: System").
- It calls `ctx.set_theme(ThemePreference::…)`.
- The choice is stored as `theme` (`system` | `light` | `dark`, default `system`) in `GuiSettings` (`crates/fleet-client/src/config.rs`). That file is read with `#[serde(default)]`, so old settings files still load.
- Apply the stored preference at startup in `main.rs` or `Root::new`, before the first frame.

## Tests

- Delete `tone_colors_match_the_web_tokens` and the `web_tokens` helper. Replace the danger-hex assertion with the new values.
- Add a test that every token differs between light and dark where it should: `bg`, `surface`, `text_high`, `fill`.
- Add a test that a theme choice saved to `GuiSettings` round-trips and that an old file without `theme` loads as System.
- Update the tests that drive the repo filter, the spawn form and the Answer button to the new controls (above). All other tests should pass unchanged. A test that fails only because a label moved means a label was renamed without this document saying so.
- `docs/ui.md` describes the web UI. Add a short egui section, or a `crates/fleet-gui/README` paragraph, naming `theme.rs` as the only place colours and sizes live.

## Acceptance

- Run the real window on Xvfb, as in `docs/superpowers/findings/egui-acceptance.md`.
- Capture the board with an agent in Needs you and one in Review, in dark and in light. Add one capture each with the side panel open, the New agent dialog open and the repo dropdown open.
- Save the captures as `docs/superpowers/findings/egui-restyle/` and compare them against the mockup.
- Check that no default-egui widget remains anywhere.
- `cargo test -p fleet-gui` and `cargo clippy` pass. The Windows-target compile guard still builds.

## Left to the builder

- How the code is split: e.g. a `widgets/menu.rs` and a `components/new_agent.rs`.
- Exact pixel nudges inside the ranges above.

- Whether the side panel's default width is 380 or 460.
- Which icon glyph the theme button uses. If Plex lacks it, draw it with the painter.
