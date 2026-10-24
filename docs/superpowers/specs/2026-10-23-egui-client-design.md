# fleet-board egui client: design (v0.1)

Status: draft for approval. Marks: **[decided]** = said by Antonio, **[my call]** = chosen by the author, open to override, **[verify]** = depends on an external fact not yet checked.

This builds on `2026-10-23-fleet-board-design.md` (daemon, contract, web UI), `docs/concept.md` and `docs/ui.md`. Nothing here changes the daemon's contract; the egui client is the second client the first design reserved room for.

## Intent and why

Antonio wants a native desktop app as the main way to use fleet-board, run on the workstation where the agents run. The web board proved the model and the schema-driven UI; the schema is centralized now, so a second front end can reuse the same description of columns, fields and actions instead of copying behaviour. The web UI stays for remote access later. **[decided]**

v0.1 is for real daily use ("ship v0.1 for initial usage and ride"): strict feature parity with the web board's v1, plus the lifecycle work a desktop app needs (starting the daemon, closing it knowingly) and a one-step install. **[decided]**

## Goals

- **Parity with the web board v1.** The five columns from the schema, cards with their fields and actions, repo filter, spawn form, answer / pause / resume / kill (kill with a second confirming click), a read-only terminal preview, a Diff tab with Refresh, live updates, a connection indicator, and error messages shown instead of swallowed. The behaviour list is `docs/acceptance.md` and the web component tests. **[decided]**
- **Rendered from the schema.** The client draws `GET /api/ui-schema` and asks no component to know a status, column or action name. The only keys it derives from an agent are `repos` and `branch` (see `docs/ui.md`). **[decided: same centralization as the web UI]**
- **The daemon outlives the window by default.** Closing the window never silently stops the daemon. When the daemon is running the user is asked what closing means, told the consequences with real numbers, and can remember the answer. **[decided]**
- **The client starts the daemon** if none is reachable, so launching the app is the only step. **[my call]**
- **One-step install** of binaries, a config template, a desktop launcher, and a check of prerequisites. **[my call: what "proper setup" means]**
- **v0.1.0 is a tagged, installable build** with the acceptance checklist run on the real `claude`. **[decided: "ship v0.1"; publishing needs a separate go-ahead, see Open decisions]**

## Non-goals

- No new feature beyond web parity: no desktop notifications, no history, no Review inbox (`TODO.md`). **[decided: parity]**
- No work on the web UI, remote access, authentication, or team mode. The web UI stays as it is. **[decided]**
- No "stop everything and kill all agents" on close; it is in `TODO.md` as a later, separate, confirmed action. **[decided]**
- Windows and macOS are not supported targets in v0.1. The daemon cannot run there (it is built on tmux and POSIX processes), so a GUI there would have nothing to talk to until there is a remote daemon (needs authentication, `TODO.md`) or a Windows-native session backend. The GUI crates are kept portable anyway, see Constraints. **[decided: Antonio wants to build on his Nobara and Windows machines; running on Windows is deferred, my call]**
- No in-process daemon inside the GUI. **[my call, see Daemon lifecycle]**

## Anti-goals (a failure even if it technically works)

- The client re-implements rules the schema or the daemon already owns, so the two front ends drift apart.
- Closing the window stops the daemon without the user having been told what that costs.
- The UI thread blocks on the network or the filesystem, so the window freezes when the daemon is slow or down.
- A card shows a status the daemon did not report. The client adds no status logic of its own.
- Company code, names or internal URLs end up in this public repo (unchanged from the main design).

## Constraints

- Rust. egui via eframe with its OpenGL backend, a native window. **[decided: egui; backend is my call]** The target machine (Antonio's Rocky 8 workstation) has an X11 session and direct OpenGL 4.6 on an NVIDIA A40 (checked 2026-10-23: `DISPLAY=:1`, `glxinfo` reports direct rendering), so no software-rendering fallback is required. **[verify]** on any other machine that the chosen backend opens a window.
- The binary runs on glibc 2.28 (Rocky/RHEL 8). Linking against system graphics or X libraries at build time must not raise that floor; load them at runtime if the toolkit allows. **[my call]**
- The client talks only to the daemon over the existing contract (HTTP JSON, SSE, loopback). It adds no daemon endpoint except where Daemon lifecycle says so.
- v0.1 targets: Linux x86_64 on Rocky 8 (the workstation, glibc 2.28 floor) and on Nobara or other current Fedora-based systems, where a plain native `cargo build --release` works and the zig cross-build is not needed. The daemon keeps x86_64 and aarch64. **[decided: Nobara; my call: the rest]**
- **macOS is the small port.** tmux, git and `claude` all exist there, so the daemon is close to portable. v0.1 does not support or test macOS (there is no Mac to test on), but the new code is written for Unix in general, not for Linux only: no dependence on `/proc`; the process-identity check uses `ps`, and `fleet-daemon`'s test helper stops reading `/proc` too. A `.app` bundle replaces the `.desktop` launcher in a later release. **[my call]**
- **Portability guard.** `fleet-client` and `fleet-gui` must compile for `x86_64-pc-windows-gnu`, and optionally for the two Apple targets. Anything Unix-only (starting the daemon detached, the pidfile, signalling a pid, the `.desktop` launcher) lives in one `cfg(unix)` module, with a stub on other platforms that reports "not supported on this platform". `build.sh` has an opt-in `FLEET_CHECK_WINDOWS=1` step that runs `cargo check` for that target. It checks that the code compiles; it does not ship or run a Windows build. **[my call]**
- Self-hosted and air-gap friendly: nothing is fetched at runtime; build dependencies come from crates.io, mirrorable through Nexus.
- License Apache-2.0; same personal repo (`TonyHeadband/fleet-board`). Antonio commits by hand; Claude commits only when asked.

## Architecture

### Crates

- `fleet-client` (new, library, no UI): everything a Rust front end needs. The HTTP calls (snapshot, schema, repos, spawn, run an action, diff), the SSE reader for events and the terminal stream, `board_reducer` (applies events to a snapshot, ignoring seq at or below the current one), and the schema helpers that exist today only in TypeScript: `column_for`, `visible_actions`, `action_request`, `field_value` (deriving `repos` and `branch`), `format_duration`. It reuses `fleet_api::eval` and the contract types directly, so there is no generated TypeScript on this side. Pure and unit-tested; no egui types. **[my call]**
- `fleet-gui` (new, binary): the egui app. Layers mirror `docs/ui.md`: a theme module (colour tokens for light and dark, tone to colour), small shared widgets (button with variants, badge, tabs, field), then board, column, card, repo filter, spawn form, preview pane (terminal and diff), the close dialog, and the daemon launcher. **[my call]**
- The daemon and `fleet-api` change only where stated below.

### Threads

The UI thread only draws. Network work runs on background threads that send messages to the UI through a channel and call `request_repaint`. A slow or absent daemon shows as "reconnecting" and never freezes the window. **[my call; it is an anti-goal if violated]**

### Data flow

Startup: ensure the daemon (below), fetch schema and repos, then open the SSE stream. The stream's first event is a snapshot; later events go through `board_reducer`. A dropped stream reconnects with `Last-Event-ID`, as the web UI does, and the daemon answers with the missed events or a fresh snapshot.

### Look

The same layout as the web board: a top bar with title, connection indicator and repo filter; the spawn form; five columns side by side, each scrolling on its own; cards with a tone-coloured edge; the preview as a side panel. Light and dark follow the system setting. Fonts and exact spacing are left to the builder. **[my call: layout; builder: details]**

## Daemon lifecycle

- **Finding it.** The client reads the daemon's address from `~/.config/fleet-board/config.toml` (`listen`, default `127.0.0.1:7878`) and `state_dir`. It must ignore keys it does not know, because the daemon's own loader rejects unknown keys, so client-only settings live in a separate file, `~/.config/fleet-board/gui.toml`. **[my call]**
- **Starting it.** If the address does not answer, the client starts `fleet-daemon` from the same directory as its own binary or from `PATH`, detached from the window, with its output appended to `<state_dir>/daemon.log`, and waits up to about 10 seconds for it to answer. If it cannot, it shows the reason and the log path. **[my call]**
- **Why not inside the window.** Claude Code's hooks report to the daemon, so a daemon that dies with the window leaves running agents untracked. A separate process keeps tracking alive. **[decided: the daemon survives the window by default]**
- **The pidfile.** The daemon writes `<state_dir>/daemon.pid` at startup and removes it on exit (a small daemon change). The client stops a daemon only through it, and only after checking that the pid belongs to a `fleet-daemon` process. It never signals a pid it cannot identify. **[my call]**
- **Closing the window** with a running daemon opens a dialog:
  - **Keep the daemon running** (default): the window closes, nothing else changes.
  - **Stop the daemon too**: sends the daemon SIGTERM through the pidfile. The dialog says what follows: the agents keep running in tmux but nothing receives their hooks, so Needs you requests, finished agents and failures are not reported; blocked agents wait; on the next start running agents show "status unverified since daemon restart" until their next event; cards, branches and worktrees are kept.
  - The dialog shows live numbers ("3 agents running: 1 working, 2 waiting for you"). A "remember my choice" box stores the answer in `gui.toml` and can be changed from a settings control.
  - When no agent is Queued, Working or Needs you, closing stops the daemon without asking, since nothing can be missed, but only if this client started it. **[my call]**
- A daemon the client did not start (started by hand) is never stopped without the dialog. **[my call]**

## Setup and packaging

- `build.sh` also builds `fleet-gui` for x86_64 into `dist/`, with the same glibc 2.28 floor, and runs the new tests. **[my call]**
- `install.sh`, shipped in `dist/`: copies `fleet-daemon` and `fleet-gui` to `~/.local/bin`; writes `~/.config/fleet-board/config.toml` from a commented template if none exists (it never overwrites one); creates the worktrees root and prints the one-time trust step from `docs/acceptance.md`; installs a `.desktop` launcher and an icon so the app can be started from the GNOME menu; checks for `tmux` 3.2 or newer and `claude` and says what is missing. It makes no change outside `~/.local` and `~/.config`. **[my call]**
- Version: the workspace version `0.1.0`; `fleet-gui --version` and `fleet-daemon --version` report it. A local tag `v0.1.0` marks the build that passed acceptance. **[my call]**

## Testing

- `fleet-client`: unit tests for the reducer, every schema helper (the same cases as the TypeScript tests, so parity is checked), SSE frame parsing including partial frames and reconnect, and the HTTP calls against a real in-process daemon router. **[my call]**
- `fleet-gui`: behaviour tests with a UI test harness for the shared widgets, the card (fields shown by condition, answer keeps literal text, two-step kill), the board (columns only from the schema), the filter, the spawn form (disabled when empty, shows the server error and keeps the prompt), the preview (read-only; diff tab; refresh; error), and the close dialog (every branch). Tests assert behaviour, not pixels. **[my call; harness choice is the builder's]**
- Acceptance: the checklist in `docs/acceptance.md`, adapted to the egui app, run on this workstation against the real `claude`, with a screenshot of the running window. **[decided: ship after real use checks]** Claude runs it, using Antonio's `claude` login as the test account for a few short prompts (**[decided]**, 2026-10-23: the goal is to improve the workplace's tooling; prompts stay generic and touch only throwaway repos). The GUI runs on a separate virtual X display (`Xvfb`) so no window opens on the desktop, and the screenshot is taken from it. There is no `xdotool` on this machine, so click-level behaviour is covered by the UI test harness and flows that need a real agent are driven through the daemon API, as in the web acceptance run.

## Calls made after the builder review **[my call]**

- **PATH for a daemon the GUI starts.** The daemon inherits the GUI's environment, so the launcher must give the GUI the same PATH as a terminal. The `.desktop` file runs the app through a login shell (`bash -lc 'exec fleet-gui'`), so `~/.local/bin` (where tmux 3.8 lives) comes before `/usr/bin`. `install.sh` runs its own prerequisite check the same way and prints the resolved path and version of `tmux` and `claude`; it fails the install with the fix if `tmux` is older than 3.2 or `claude` is missing. If the daemon still cannot start, the GUI shows the last lines of `<state_dir>/daemon.log` (which carry the daemon's own reason, for example "tmux 2.7 is too old"), not a bare timeout. No `tmux_command` config key and no other daemon change is added for this in v0.1; the pidfile is the only daemon change.
- **Native build fallback.** `build.sh` builds `fleet-gui` for x86_64 with the glibc 2.28 zig build first. If that cannot produce a working binary, it falls back to a native `cargo build --release` on the Rocky 8 workstation (whose glibc is the floor) and prints which one it used; it does not fail the whole script. The daemon's two-target build is unchanged.

## Left to the builder

- egui and eframe versions and feature flags; the UI test harness.
- Fonts, spacing, exact colours (the tokens must map to the web UI's variables so both read alike).
- Thread and channel structure, the SSE parser, HTTP library.
- How the pidfile and process-identity check are implemented.
- Icon artwork (a simple generated mark is enough).

## Open decisions

- **Publishing v0.1.** The tag `v0.1.0` is created locally by Claude after acceptance passes **[decided]**. Pushing the commits and tag and attaching binaries to a GitHub release are outward-facing and still need Antonio's go-ahead when the build is ready.
- Whether `install.sh` should offer a systemd user service for the daemon. v0.1 does not install one. **[my call: not in v0.1]**
