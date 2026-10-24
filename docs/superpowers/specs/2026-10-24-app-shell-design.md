# App shell: drawer, views, settings, footer messages and one logger — design

Date: 2026-10-24. Scope: a new `fleet-log` crate, `fleet-daemon`, `fleet-client`, `fleet-gui`. The web UI stays frozen.
Builds on `2026-10-24-folder-picker-design.md`, which must land first (Settings reuses its folder mode).
Another session is implementing the folder picker and other egui UI work in this checkout right now (uncommitted
changes across `fleet-gui`, `fleet-daemon`, `fleet-api`, new `crates/fleet-fs`). Start this work only after that
is committed, from that commit, in its own worktree/branch; re-read `app.rs`, `shell.rs` and `components/` then,
because the line references below are from commit 137c19d and will have moved.
Mockups: `.superpowers/brainstorm/825084-1792854790/content/layout-v5.html` (gitignored; v2 settles the row
heights, v5 the tab bar, log tab and footer history).

## Why

The egui client is one fixed layout: a top bar crammed with the title, the live dot, the repo filter, the theme
toggle and New project, the board, the preview panel, and a footer that exists only to show a remembered close
choice. There is nowhere to put a second view (swim lanes, a per-agent view) or a settings page, and errors land in
one red line under the top bar that the next action overwrites.

Antonio wants a structured frame first, so later views drop in without reshuffling the UI again. This round is
**restructuring only**: Kanban stays the main and only board view; Settings is the one new page.

Logging today is 19 `eprintln!` calls in the daemon, captured into `daemon.log` only when the GUI started the
daemon, with no timestamps or levels, and nothing in the GUI. "Establish one way to log", so the log viewer works
on both.

## Goals

### Layout

The window splits in two: the **drawer** on the left, the **content** on the right.

- **Drawer** (the same on every view, owned by the app shell):
  - **Drawer header**: the brand row ("fleet-board").
  - **New project** button (moved here from the top bar; it applies to every view).
  - **Nav**: one entry per view. This round: **Kanban**, then a separator, then **Settings**. Kanban is selected at
    start. No placeholder entries for future views.
  - **Drawer footer**: one line, the live status: dot + `live` / `reconnecting…`, then `N agents · M need you`
    (`need you` = agents in the attention/Needs-you state; omit that part when M is 0).
- **Content** (changes with the selected nav entry):
  - **Header**: owned by the view. Kanban: view title + repo filter. Settings: view title only.
  - **Main**: the view itself. A view may put a **tab bar** directly under the header, at the top of Main.
  - **Detail**: the right column (today's preview panel). Only on agent views. Not shown on Settings.
  - **Footer**: one line, app-wide, the message line (below). Shown under every view.

**Alignment (hard requirement, it looked amateur without it):** the drawer header and the content header are the
same height and their bottom border is one continuous line across the window. The drawer footer and the content
footer are the same height and their top border is one continuous line. Use shared height constants in `theme.rs`.

### Views

- A view decides its header contents, its Main, and whether it shows Detail. The drawer and the footer are not the
  view's business.
- The selected agent (today `FleetApp::preview`) belongs to the app, not to the board, so it survives switching
  views and a future agent view shows the same agent. Switching to Settings hides Detail; switching back to Kanban
  shows it again for the same agent. The pane stream may stay open while hidden (builder's call).
- The selected view is not persisted across restarts; the app always opens on Kanban.

### Tab bar

- A reusable component: a view hands it `(id, label)` pairs and the selected id and gets back a click. It is the
  existing `widgets::tabs` drawn in a full-width strip with a bottom border, directly under the header.
- Settings uses it with three tabs: **General**, **Daemon**, **Log**. General is selected when Settings opens; the
  last tab is remembered while the app runs.

### Settings → General

- **Theme**: System / Dark / Light, a segmented choice. Saved to `gui.toml` immediately. The theme icon button in
  the top bar goes away.
- **Closing the window**: Ask / Keep daemon running / Stop daemon. "Ask" is `close_choice = None`. Saved
  immediately. The close-choice bottom panel (`CloseFlow::settings_ui`, "Ask me again") goes away. The close dialog's
  "remember" checkbox stays and writes the same setting.
- **Projects folder**: text box + **Browse** (the folder picker's folder mode) + **Save**. Note under it: "Applies
  to new projects only. Saving restarts the daemon; running agents keep going." Save:
  1. validates like the first-run screen (`Root::save_first_run`: expand `~`, absolute, creatable);
  2. is disabled, with the reason as a tooltip, while any agent is Queued (a spawn whose tmux session may not exist
     yet would be marked Dead by the startup reconcile) or while this window has a spawn, add-repo or grant call in
     flight (a clone in progress would be swept at startup); no new daemon endpoint for this. An attach the agent
     started itself cannot be seen from the GUI; that residual risk is accepted (Claude's call);
  3. if any agent has a pending repo request, asks first: it will be marked "interrupted by daemon restart"
     (`agents.rs:684`);
  4. writes `projects_root` into the daemon's `config.toml`, keeping every other key and comment;
  5. stops the daemon (`stop_daemon`) and starts it again (`ensure_daemon`), off the UI thread, with progress in
     the footer;
  6. reconnects the event stream; agents are re-adopted by the daemon's startup reconcile (`agents.rs:833`).
  - If this window did not start the daemon (`Started::AlreadyRunning`), Save still restarts it; afterwards the
    window owns it (we started it). Stopping needs the pidfile; with no pidfile, Save writes the config and says in
    the footer that the change applies when the daemon is restarted.
  - Trust: the first-run screen shows the `claude` trust command for the root; General shows the same line and Copy
    button for the new folder.

- **Footer**: "Keep errors and warnings in the footer until dismissed", a checkbox, default on, saved immediately
  (see Footer message line).

### Settings → Daemon

Read-only: address (`listen`), who started it (this window / already running), state directory, config file path,
pid (when the pidfile exists).

### Settings → Log

- A **source** switch: **App** / **Daemon** / **Both** (Both merges by timestamp). Default: Both.
- A search box (case-insensitive substring over the whole line) and one toggle per level: error, warning, info,
  debug. All on except debug.
- The list: time, coloured level dot, target in low text, message. Newest at the bottom; follows new entries
  unless the user has scrolled up. Monospace.
- App entries: `gui.log` read when the tab opens, then appended live from the logger. Daemon entries: `daemon.log`
  read when the tab opens and its new tail re-read while the tab is visible (once a second is fine).
- Lines that do not parse (panics, old `eprintln!` output, the stderr safety net) are shown as-is, with no level,
  attached to the time of the previous parsed line; level toggles do not hide them.

### Footer message line

- One line, app-wide. It replaces `FleetApp::problem` and its red line under the top bar. The New project dialog
  keeps its own error line (that error belongs in the dialog).
- Messages have a **level** with a coloured dot: **error** (red, `danger`), **warning** (amber, `Attention` tone),
  **success** (green, `Done` tone), **info** (neutral, `text_low`).
- Messages are queued in arrival order and **cycle**: each shows for 4 s (errors 8 s), then the next. A position
  counter `i/n` shows while more than one is queued.
- **Success and info show once**, then leave the queue (they stay in the history).
- **Errors and warnings keep looping until clicked** when the General setting "Keep errors and warnings in the
  footer until dismissed" is on (default on, saved in `gui.toml`). Clicking the line dismisses the message shown if
  it is a looping one, and skips to the next otherwise. With the setting off, errors and warnings show once like the
  rest. (Antonio)
- When the queue is empty, the last message stays, dimmed, until a new one arrives.
- An identical message (same level and text) already queued is not queued twice.
- Right end: a control that opens the **history**: the last 10 messages, newest first, with time and dot, and an
  "Open log →" link that switches to Settings → Log.
- What posts to it: every place that sets `problem` today (actions, spawn failure out of sight, add repo, grant /
  refuse, theme save), plus successes that are silent today: agent started, repo added, repo granted, setting
  saved, daemon restarted. Stream reconnects are warning/info in the log only, not the footer (the drawer footer
  already shows them).

### One logger: `fleet-log`

A new workspace crate used by `fleet-daemon`, `fleet-client` and `fleet-gui`.

- Built on **`tracing` + `tracing-subscriber`** (Antonio, chosen so OpenTelemetry can be added later as one more
  layer without touching call sites again). `tracing` 0.1 is already in `Cargo.lock`; `tracing-subscriber` and
  `tracing-log` are new and must be mirrored into Nexus — check them before the first build. Call sites use
  `tracing::error!/warn!/info!/debug!`. eframe/winit/wgpu's own `log` output is captured with `tracing-log`'s
  `LogTracer`. Rejected: the `log` facade (logs only; adding traces later would mean changing every call site
  again) and home-made macros.
- The file writer, the subscriber fan-out and the stderr writer are each a `tracing-subscriber` `Layer`, so a
  later OTel layer slots in next to them.
- `fleet_log::init(file, level)` installs the global subscriber: appends to `file`, one line per record, flushes per
  line. Line format, shared by both files so one parser reads them:

  ```
  2026-10-24T14:09:55Z ERROR fleet_gui::app Could not start the agent: repo "web" not found agent_id=7f3a project=fix-login
  ```

  UTC, second precision, level padded to 5, target, message; embedded newlines escaped so one record is one line.
- A subscriber API for the GUI: the logger also fans out each record to in-process subscribers (a channel) so the
  Log tab and the footer read the same stream. A parser for the line format lives in the same crate.
- **Footer flag**: a record reaches the footer only when the call marks it for the user (e.g. a key-value
  `notify = true`, or a dedicated `fleet_log::notify(level, msg)`). Success is a footer level; in the file it is
  written as `INFO` with the notify marker so the level set stays `tracing`'s. Exact mechanism is the builder's call.
- Default file level: info for both; `FLEET_LOG` (an `EnvFilter` directive, e.g. `debug`) overrides it. Third-party crates (eframe, winit, wgpu, hyper…)
  are capped at warn.
- **Files**: `gui.log` and `daemon.log` in the state directory (`ClientConfig::state_dir`). The daemon now writes
  `daemon.log` itself through `fleet-log`, so it is correct however the daemon was started. The GUI keeps
  redirecting the daemon's stdout/stderr when it starts it, into `daemon.log` as today, as a safety net for panics
  (unparsed lines, handled above).
- **stderr** (Claude's call): the logger also writes each line to stderr **only when stderr is a terminal**, so
  `fleet-daemon` run by hand still shows its log, and a GUI-started daemon (stderr redirected into `daemon.log`)
  does not get every line twice. Errors before `init` (bad config, tmux check, bind failure) go to stderr, which the
  GUI's redirect lands in `daemon.log` as unparsed lines; the Log tab shows those as-is (see Settings → Log).
- **Structured fields, not formatted text**: identifiers go in as fields (`agent_id`, `project`, `repo`, `action`),
  e.g. `info!(agent_id = %id, repo = %name, "repo added")`, so they become OTel attributes later. The file line
  renders them after the message as `key=value` pairs; the Log tab's search covers them.
- **Spans where a later trace would want one**, with no exporter now: one span per daemon HTTP request (a
  hand-written middleware; `tower-http` is not in the lock, so no `TraceLayer`),
  spawn, attach/clone, reconcile pass. Spans are not written to the files (events only), so the log format does
  not change.
- **`[telemetry]` in `config.toml`**: `otlp_endpoint` (optional string), parsed by the daemon and ignored, with a
  debug line saying telemetry is not built yet when it is set. Reserves the key so the later work does not need a
  config migration.
- **Daemon switch**: every `eprintln!` in `crates/fleet-daemon/src` becomes a `tracing::` call at the right level;
  user-facing CLI errors at exit (`main.rs:116`) still go to stderr as well.
- `fleet-client`: background failures that are logged today nowhere (stream errors, reconnects) log at
  warn/debug.

## Non-goals

- Swim lanes, per-agent views or any second board view. The frame must allow them; none is built.
- Log trimming or rotation. Files grow until #22 lands.
- Persisting the selected view or Settings tab across restarts.
- Changing `listen` or `state_dir` from Settings (read-only in Daemon).
- A collapsible / icon-rail drawer. Fixed width.
- The web UI.

## Anti-goals

- Header/footer rows of the drawer and content that do not line up. The continuous lines are the point.
- A second way to log. No `eprintln!`/`println!` diagnostics left in daemon, client or GUI code (CLI output meant
  for a user at a terminal is not diagnostics).
- Footer noise: debug lines, stream flaps or third-party warnings cycling through the footer.
- Losing an error because a later message replaced it before it was seen (today's `problem` behaviour).
- Restarting the daemon without saying so, or losing running agents on restart.

## Decided

- Drawer + content split; section names: drawer header, nav, drawer footer; content header, main (with optional
  tab bar), detail, footer. (Antonio)
- Kanban stays the default view. (Antonio)
- Daemon info lives in Settings, not the drawer; the drawer footer is the live status. (Antonio)
- Header/footer heights and lines aligned across drawer and content. (Antonio)
- Footer cycles messages with a coloured dot per level; history of the last 10; errors and warnings loop until
  dismissed when the setting is on, otherwise everything shows once. (Antonio)
- Log tab in Settings with search; persisted to `gui.log`; shows `daemon.log` too. (Antonio)
- Tab bar under the content header, reusable across views. (Antonio)
- One shared logger crate; the daemon switches to it in this round. (Antonio)
- Projects folder editable in Settings, reusing the folder picker. (Antonio)
- Calls made by Claude during the conversation, open to change: New project in the drawer; 4 s / 8 s cycle
  timing; last message stays dimmed; `tracing` with OTel prewiring (Antonio); App/Daemon/Both source switch; Save restarts the
  daemon; theme and close choice saved immediately while the projects folder needs Save.

## Left to the builder

- Module layout in `fleet-gui` (a `View` enum vs a trait; where the shell lives relative to `shell.rs`/`app.rs`).
- Drawer width, icons for nav entries (`widgets::icon` has the set), exact row heights (as long as they are shared).
- How the footer queue and history are stored and tested; how the `notify` flag is carried.
- Editing `config.toml` while keeping comments (e.g. `toml_edit` if already in the lock, otherwise a targeted line
  rewrite); flag a new crate before adding it (air-gapped Nexus).
- Timestamp formatting without a new dependency if `time`'s formatting feature pulls one in.
- `toml_edit` 0.25 and `time` 0.3 are already in `Cargo.lock`.
- The mockups are gitignored, so a new worktree will not have them: read them from
  `/mnt/users/afrontino/fleet-board/.superpowers/brainstorm/825084-1792854790/content/`.
- Order of work. Suggested: `fleet-log` + daemon switch first, then shell/drawer/views, then footer, then Settings.

- No OTel crates (`opentelemetry*`) in this round.

## Follow-ups (filed)

- #22 Trim the log files: keep only the last N working days (e.g. delete entries older than 2 working days) — deferred
  by Antonio, 2026-10-24.
- #23 Repos under the **old** projects root are no longer refused as `InsideProjects` after the root moves
  (`repos.rs:180` knows only the current root).
- #24 OpenTelemetry export: an OTLP layer in `fleet-log` honouring `[telemetry] otlp_endpoint`, daemon metrics (time
  per status, Needs-you wait, clone/fetch time, reconcile time, HTTP latency), and Claude Code's own telemetry
  passed through the env the daemon gives each `claude` (`CLAUDE_CODE_ENABLE_TELEMETRY`, `OTEL_*`,
  `OTEL_RESOURCE_ATTRIBUTES` with agent id and project) — deferred by Antonio, 2026-10-24.
