# How the UI is put together

The UI draws whatever the daemon's `UiSchema` describes (`GET /api/ui-schema`; defined in `crates/fleet-api/src/ui.rs`). Components hold no status, column or action names. Add or change what the board shows in the schema first; touch React only for a new kind of thing.

## Layers

| Layer | Where | Job |
|---|---|---|
| Schema logic | `web/src/api/schema.ts` | `evalCondition`, `columnFor`, `visibleActions`, `actionRequest`, `fieldValue`, `formatDuration`. Pure functions, no React. |
| State | `web/src/api/board.ts`, `events.ts`, `hooks.ts` | `boardReducer` applies events; `subscribe` and `useFleet` feed it from SSE. |
| Primitives | `web/src/ui/` | `Button`, `Badge`, `Tabs`, `Field`. The only place a raw `<button>` or a badge `<span>` is written. |
| Components | `web/src/components/` | `Board` → `Column` → `Card`, plus `RepoFilter`, `SpawnForm`, `PreviewPane`. They compose primitives and read the schema. |
| Styles | `web/src/styles.css` | The only stylesheet. Colours are variables on `:root` (with a dark-mode override); tone and variant come from `data-tone`, `data-variant` and `aria-pressed` attributes. |

## Rules

- A new card field is a `FieldDef` in the schema. `Field` draws it by kind (`text`, `badge`, `duration`, `mono`) and renders a list value as one element per entry.
- A new button action is an `ActionDef` in the schema. `client.run` executes it; no component names an action.
- A new column is a `ColumnDef`. `Board` renders it with `Column`.
- `fieldValue` derives exactly two keys from the workspaces: `repos` and `branch`. A contract test in `fleet-api` rejects any other card field that is not an `Agent` field.
- Use `Button` and `Badge`, never a raw `<button>` or a `.badge` span. Style a new look as a `data-variant` in `styles.css`, not as a new class.
- No inline `style=` props and no per-component stylesheets.
- Inputs, selects and textareas are plain elements styled by element selectors in `styles.css`; they have no behaviour of their own to share.

## Tests

`pnpm --dir web test`. Component tests query by role and title, never by class, so a restyle does not break them. The default schema the tests use is `web/src/api/generated/default-ui-schema.json`, written by `cargo test -p fleet-api`, so the UI is always tested against the daemon's real default.

## egui client (`crates/fleet-gui`)

- The window is a **drawer** (left, `drawer.rs`) and the **content** (right). The app (`app.rs`) owns the drawer (brand row, New project, nav, live status), the content footer and the selected agent; a view owns the content header, its main area and whether the detail panel (the selected agent's preview) shows. The selection outlives a view that hides it.
- The drawer header and the content header are both exactly `theme::layout::HEADER` tall, the drawer footer and the content footer exactly `layout::FOOTER`, so the border under the headers and the border over the footers each run unbroken across the window. Every row's border is its panel's own separator line. `tests/shell_layout.rs` checks this from measured rects and painted pixels; keep it passing.
- `views/` holds one module per view (`kanban.rs`, `settings/`). A new view is a `View` variant, a module in `views/` with its header and main, and a nav entry (`drawer::VIEWS`). The app always opens on Kanban.
- A view that needs tabs puts `widgets::tab_bar` (a full-width strip with a bottom border) at the top of its main area, directly under the header. Settings does, with General, Daemon and Log; its tab is kept while the app runs.
- Settings → General applies Theme, Closing the window and Footer at once (saved to `gui.toml`). Its Projects folder needs Save, because Save writes `projects_root` into the daemon's `config.toml` (`config::set_projects_root`, which keeps the user's other keys and comments) and restarts the daemon. Settings → Daemon shows the address, who started the daemon, the state directory, the config file and the PID (when `daemon.pid` exists), with Restart daemon.
- Both restarts go through `FleetApp::start_restart`: one background thread writes the config (Save only), then `shell::restart_daemon` (stop, then ensure). The app gets them from `shell::DaemonCtl`, which `Root` builds from its own `ensure`/`stop` when the board starts; an app without one (tests, embedding) leaves those settings out. Save and Restart daemon are disabled, with the reason as a tooltip, while an agent is Queued or a spawn, add-repo or grant is in flight (the daemon's startup reconcile would break them), and Save also for the current folder. A pending repo request asks first (`Restart anyway` / `Cancel`). After a restart the window owns the daemon: `take_restarted()` lets `Root` tell its `CloseFlow`.
- Settings → Log (`views/settings/log.rs`) lists the app's and the daemon's lines in one list. The files are `gui.log` and `daemon.log` in the daemon's state directory (`DaemonCtl.state_dir`; an app without one lists only what it logged itself). The source switch is App, Daemon or Both (default), merged by time with the app's line first on a tie; error, warning, info and debug toggles (debug off by default; trace goes with debug) leave stray lines such as a panic visible; the search is a case-insensitive substring of the whole rendered line, fields included. Opening the tab reads at most the last `TAIL` (1 MiB) of each file from a line start on a background thread; a missing file says `No daemon log at <path>` or `No app log at <path>`. The app's lines then come from `FleetApp::log_entries()`; file lines are listed only when older than the first live entry, so none appears twice. The daemon's file is re-read from its last offset once a second, only while the tab is drawn (`request_repaint_after(1 s)` only then, no file IO per frame), holding back a half-written last line and starting over when the file shrinks. The merged list is rebuilt only when its inputs change.
- The content footer (`components/footer.rs`, state in `messages.rs`) shows one message line at a time from a queue: success, info and warning for 4 s, error for 8 s, in arrival order. With Settings → General → Footer on (the default, `sticky_alerts` in `gui.toml`) an error or warning loops back into the queue until the user clicks it away; a click on any other message skips it. When the queue empties the last message stays, dimmed. The line is one row with the full text as its tooltip; the `History` button lists the last 10 (time in UTC) and ends in `Open log →`, which opens Settings → Log.
- To tell the user something, log an event with a `notify` field of `error`, `warning`, `success` or `info` from anywhere in the GUI: `tracing::error!(notify = "error", agent_id = %id, "{e}")`. The message is the footer text and ids go in fields. Only events whose target starts with `fleet_` count, so a dependency's `notify` field never reaches the user. `FleetApp` collects every such event from `fleet-log` and feeds the queue; there is no per-view error state. A form that has its own error line (the New project dialog) keeps it and logs nothing extra.
- The window stays idle while the footer is on screen: it asks for a repaint only at the next turn of the queue (`Messages::next_change`), and not at all when a looping message is alone.
- The app's name as a person reads it is `theme::APP_NAME` ("Fleet Manager"); ids, paths and binaries stay `fleet-manager`.
- `theme.rs` is the only place for colours, type sizes, corner radii and fonts. Light and dark come from one token set; the rest of the crate reads tokens and never writes a literal colour.
- Spacing (margins, gaps, padding) is not tokenised: each component sets its own, next to the layout it belongs to.
- `widgets/` is the only place buttons, badges, tabs, chips, icons, dots and nav entries are drawn. A new look is a variant of a primitive there, not a styled one-off in a component.
- egui's stock widgets (`ComboBox`, `checkbox`, `TextEdit`, `ScrollArea`, tooltips) are not redrawn in `widgets/`: they pick up the tokens through the `Visuals` that `theme::apply` installs, and components call them directly.
- Components in `components/` compose those primitives and read the schema. They never write a colour or a font size.
- `components/folder_picker.rs` is the one folder and repo picker: a `Browse…` button whose modal lists folders (and a repo's checkouts) from `fleet-fs` on background threads. Every text box that takes a folder or repo reuses it; its rows are the `list_row` primitive.
- The fonts (IBM Plex Sans Regular and Medium, IBM Plex Mono Regular) are vendored under `crates/fleet-gui/assets/fonts/` with their SIL OFL licence and compiled in with `include_bytes!`, so the binary runs air-gapped.
- The window must stay idle: no repaint loop beyond the 15 s refresh, events and background results. Motion goes only through `theme::animate(ctx)` (with `theme::pulse` built on it): it is true only while the window has focus, and then frames are throttled to about 100 ms (`theme::ANIMATION_FRAME`). It is false while the window is unfocused and in every test (`egui_kittest`'s `Harness::run()` panics on continuous repaints).
- Tests (`cargo test -p fleet-gui`) find widgets by role and label, so a restyle does not break them; keep visible labels unchanged.
