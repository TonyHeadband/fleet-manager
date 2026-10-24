# egui restyle acceptance

First run 2026-10-23 (Task 9); re-captured 2026-10-24 after the final fix wave. This file describes the 2026-10-24 state. Captures from the first run that were not retaken are marked as such below.

## How it was run

- Rocky 8 workstation, release build of `fleet-gui` (`cargo build --release -p fleet-gui -p fleet-daemon`), in a window on `Xvfb :<free display> -screen 0 1400x800x24 +extension GLX -nolisten tcp` (software GL). Captured by window id with `import -window`.
- **No real `claude`, no real agents.** A throwaway `fleet-daemon` with:
  - a temp `config.toml`, a temp `state_dir`, and three temp git repos (`portal`, `core`, `atlas`);
  - a private tmux socket (`FLEET_TMUX_SOCKET`). Its directory (`TMUX_TMPDIR`) was a `mktemp -d /tmp/fb-tmux.XXXXXX`, because a Unix socket path must stay under about 108 bytes and the session's scratch path is longer;
  - `claude_command` set to a stub script that prints a short Claude-like transcript and then does `exec sleep 3600`.
- Cards were put into states by posting synthetic hook events to `/internal/hooks/<id>` with each agent's token, read from the temp `fleet.db`:
  - Working: `PreToolUse`;
  - Paused: the pause API;
  - Needs you: `PermissionRequest` gives `permission`, and a `Notification` of type `elicitation_dialog` gives `question`;
  - Review: `Stop`.
- "Has changes" comes from files written into the worktrees. The daemon only recomputes it while an agent is in Review, so the Needs-you card was sent through `Stop` first and then `PermissionRequest`.
- **The two-repo card.** The API has no multi-repo spawn, so before any client connected a real `git worktree` of `core` was added to that agent's stored record as a second workspace (`json_insert` into `agents.data`). Clients read it from the snapshot, and the daemon's reconcile treats it like any other workspace.
- `fleet-gui` ran with `HOME` set to a temp dir (its own `config.toml` and `gui.toml`). The theme was set by writing `theme = "dark"` or `"light"` to `gui.toml` before launch, and the owner's real config was never read or written. The GUI found the daemon already up, so it did not start or own one.
- **Clicks.** `xdotool` is missing, so a small Python `ctypes` script drove `libXtst` instead: XTEST pointer and key events, plus `XSetInputFocus` to give the window focus so typing reaches it.
- Only what the run started was stopped afterwards: the GUI, the daemon and Xvfb (by recorded PID), and the private tmux server with its stub sessions. The temp dirs were then removed.

## Captures (`egui-restyle/`)

Retaken 2026-10-24 (after the fix wave):

| File | Shows |
|---|---|
| `board-dark.png`, `board-light.png` | Board with five cards (described below) |
| `panel-dark.png`, `panel-light.png` | The side panel open on the two-repo Needs-you card: Terminal tab with the stub's transcript, and the reply box with "Yes, go ahead" typed (window focused) |
| `new-agent-dark.png`, `new-agent-light.png` | New agent dialog, empty prompt (Start agent disabled) |
| `new-agent-typed-dark.png`, `new-agent-typed-light.png` | New agent dialog with a prompt typed, over the open side panel |
| `zoom-card-dark.png` | 3x crop: the two-repo card |
| `zoom-working-dark.png` | 3x crop: a Working card, showing the activity dot |
| `zoom-colhead-dark.png` | 2x crop: column headers under the top bar |
| `zoom-selection-dark.png` | 2x crop: a drag selection in the Terminal tab |

The board in `board-*.png` has five cards:

- Working, twice: one running Edit, and one Paused running Read.
- Needs you, twice:
  - `question`;
  - one card with two repos (`portal core`), a `permission` badge and a Has changes badge.
- Review, once, with Has changes.

From the first run (2026-10-23), **not retaken**:

- `panel-diff-*.png`, `repo-menu-*.png`, `repo-menu-ticked-*.png`, `close-dialog-*.png`, `startup-error-dark.png`, `footer-dark.png`.
- `zoom-send-enabled-dark.png`, which shows the card's Answer and Send control. Task 11 removed that control, so this capture is historical only.

These first-run captures still show the old board frame: column headers inset 8 px, with no rule, and dividers stopping short. The dialogs and menus in them are otherwise the current styling.

Never captured, in either run:
- the "Starting the daemon…" screen (too brief);
- the "Stopping the daemon…" modal and the "The daemon did not stop" modal, which need a stop that hangs or fails;
- the light footer.

All three screens are built from the same parts as the captured ones:
- the "did not stop" and "Stopping" modals use `components::modal_frame`, the heading style and the shared `button` primitives (`crates/fleet-gui/src/shell.rs:144-162`);
- "Starting" uses `centred_card` (`shell.rs:246`).

They are therefore expected to match the close dialog, but nobody has looked at them.

## Measured on the 2026-10-24 captures

Both themes gave the same geometry:

- **Top bar.** Its 1 px `border` is at y=46.
- **Column headers.** The `surface` starts at y=47 (flush) and spans the full column from x=0. The header's bottom rule is one solid `border` row at y=84, then the body (`bg`) starts at y=85.
- **Dividers.** Each column divider is `border` from y=47 down to y=799, the last pixel row of the window.
- **Two-repo card.** It reads `portal core` with a clear gap. The `permission` and Has changes badges sit about 6 px apart (`zoom-card-dark.png`).
- **Task titles.** They wrap between words: "Refactor the retry policy so transient 502 / errors back off exponentially". They are left-aligned, not justified. The two-line ellipsis still cuts at the width, for example "stable …".
- **Working dot.** It sits on the middle of its "running Edit" line (`zoom-working-dark.png`). The paused card's dot is static.
- **Selection.** Selected terminal text is a clear mid-grey band with `text_high` text (`zoom-selection-dark.png`).
- **Caret.** It is a 2 px `text_high` bar: white in dark, black in light.
- **New agent dialog.** It is 440 px wide (x=480..920), with a 6 px radius, on the shared modal frame.
- **Reply box.**
  - It is pinned to the bottom of the panel, under a 1 px top border.
  - A 3-row field sits above a row with "Ctrl+Enter" on the left and Send on the right. No control is joined to another, so there is no seam or height step.
  - Send is disabled while the box is blank.

## Remaining differences from the mockup

Layout and controls:
1. **The panel ✕ and the dialog ✕** are bordered 26 px square icon buttons. The mockup draws a bare glyph.
2. **The New agent dialog has no rules** under the header or above the footer, as the mockup has. Its Repo field is a full-width `ComboBox` with egui's right-hand caret, not the mockup's inline "core ▾".
3. **Focused text fields** (the prompt and reply box) get a bright 1 px border (near `text_high`) from egui's own focused-input styling. The mockup's focused prompt uses `text_low`.
4. **The panel's tab labels** are inset about 8 px from the panel's header text.
5. **The side panel** defaults to about 420 px. The mockup is 380, and the spec allows 380–460.
6. **The modal backdrop** is egui's default dim, about 39 % black in both themes. The mockup uses 25 % in light and 55 % in dark, so the light backdrop is heavier than the mockup's.
7. **The top bar.**
   - It is about 46 px tall, against the mockup's 49.
   - The theme button is a painted disc (half-filled in dark, solid in light) rather than "◐".
   - "New agent" has no "+", because labels were kept unchanged.
8. **In the footer** (first-run capture only), "Ask me again" sits about 2 px below the status text.
9. **The repo menu** (first-run capture only):
   - its checkboxes are egui's outlined tick, not the mockup's filled box;
   - "All repos" shows ticked when nothing is selected, where the mockup leaves it empty.

Text and content:
10. **The badge text** is the schema's `permission` and `question`, not the mockup's "Permission needed". This is by design: labels stay unchanged.
11. **Pause and Resume are borderless ghost buttons**, where the mockup draws outlined ones. The spec says ghost and the spec wins, so this is intended.
12. **The Terminal tab is plain text in Plex Mono.**
    - It has none of the mockup's coloured bullets or hunk colours; the diff tab does have them.
    - `⎿` is drawn as a tofu box, because neither Plex Mono nor egui's fallback fonts carry it. That is the intended `Tofu` policy, but it shows on real Claude output.

## Where egui's default look still shows

- **`TextEdit` focus outline.** This is egui's focused-input styling, not set from a token (see 3).
- **`ComboBox`.** The repo filter's popup frame, and in the New agent dialog the closed box and its caret, are egui's widget on the tokens (`components/repo_filter.rs`, `components/new_agent.rs`).
- **`checkbox`.** The repo menu rows and "Remember my choice" use egui's checkbox on the tokens (see 9).
- **The `ScrollArea` bar.** In the Terminal and Diff panes it is egui's thin floating bar. The pane content in these captures was short, so no bar showed; the first run saw it as a 2 px mid-grey bar.
- **Tooltips.** They use egui's tooltip frame, with the token shadow and radius.

Fixed in this wave and no longer default: the blue text caret, and the near-invisible selection (`raised` on `bg`, about 1.15:1).

No egui-grey pill button, unstyled window frame or default-coloured widget appears on any captured screen. The board, side panel, reply box, New agent dialog and repo menu are all on the token palette in both themes. The controls listed above are stock egui widgets styled through `Visuals`, as the spec intended, but they are visibly egui-shaped. They are not the mockup's drawings.

## Other notes

- **Startup error with a daemon already running.**
  - What happens: `fleet-gui` looks for the daemon binary before it checks whether a daemon is already answering. If a daemon is up but `fleet-daemon` is neither beside the GUI nor on `PATH`, the GUI shows "could not start its daemon".
  - This behaviour predates the restyle (`crates/fleet-gui/src/main.rs:41`, `daemon_control.rs`).
  - This run built `fleet-daemon` next to `fleet-gui`, so it did not hit the error.
- **Trailing blank rows in the Terminal tab.** tmux returns the whole pane, including the empty rows under short output, and the Terminal view sticks to the bottom. Since the reply box made the pane shorter, those empty rows were all the view showed. The client now trims trailing blank rows (`components/preview.rs`).

## Binary size

| Binary | Size |
|---|---|
| `dist/fleet-gui-x86_64-linux` before the restyle (zig, glibc 2.28, stripped) | 10,362,976 B |
| native `cargo build --release -p fleet-gui` after the fix wave, stripped | 11,017,624 B |
| difference | +654,648 B (about +639 KB) |
| of which the three vendored fonts | 558,900 B |
| **of which everything else (widget, theme and component code)** | **+95,748 B (about +94 KB)** |
| native release, not stripped | 15,002,328 B |

The comparison mixes a zig build with a native one, so treat the non-font figure as accurate to a few tens of KB. The total is within the spec's expected 600–800 KB.

## Guards and tests

- `cargo test -p fleet-gui -p fleet-client`: 175 passed, 0 failed.
- `cargo clippy -p fleet-gui -p fleet-client --all-targets -- -D warnings -A clippy::large_enum_variant`: clean.
- `cargo check -p fleet-gui --target x86_64-pc-windows-gnu`: ok.
- No colour literal outside `theme.rs`. The grep for `Color32::(from_rgb|WHITE|BLACK)` and hex values prints nothing.

## Verdict

The board now has the mockup's structure in both themes, which was the biggest gap after the first run:
- flush, full-width column headers with a bottom rule;
- dividers that run from the top bar to the window's bottom edge;
- cards whose repos and badges are spaced;
- titles that wrap at words, unjustified;
- a centred Working dot that stays still while paused;
- readable selections and a neutral caret.

The side panel with its reply box and the New agent dialog look like finished screens, with no seams, notches or blank panes.

It is not a pixel match. These still read as egui rather than the mockup:
- the bordered ✕ buttons;
- the dialog without header and footer rules;
- the full-width `ComboBox`;
- the bright focus outline;
- egui's checkbox and scroll bar;
- the heavier light backdrop.

Differences 1–6 and 9 are the remaining work, if the goal is a closer match. They are all within `widgets/`, `theme.rs` and the two dialogs. The close dialog, footer, repo menu and diff tab were not re-captured after this wave, and the "Starting", "Stopping" and "did not stop" screens have never been captured.
