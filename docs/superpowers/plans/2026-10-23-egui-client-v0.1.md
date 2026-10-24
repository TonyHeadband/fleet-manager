# fleet-board egui client v0.1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A native egui desktop client with strict parity to the web board, a daemon lifecycle that asks before stopping the daemon, a one-step install, and a tagged v0.1.0 that has passed acceptance on the real `claude`.

**Architecture:** Two new crates. `fleet-client` is a UI-free library: HTTP calls, SSE reader, board reducer, schema helpers (ports of `web/src/api/*.ts`), config and daemon control. `fleet-gui` is the egui app: theme, shared widgets, board/card/column, filter, spawn form, preview (terminal and diff), app state, close dialog. Both use `fleet-api` types directly and the daemon's `UiSchema`; the daemon changes only by writing a pidfile and handling SIGTERM.

**Tech Stack:** Rust 1.95+, eframe/egui 0.37 (glow backend, X11 and Wayland), egui_kittest 0.37 for UI behaviour tests, ureq 3 for HTTP, std threads and mpsc channels, libc for signals (unix only).

**Spec:** `docs/superpowers/specs/2026-10-23-egui-client-design.md`. Read it, `docs/ui.md` and `docs/acceptance.md`. The web components in `web/src/` and their tests are the behaviour reference for parity. Claude commits only when asked; this plan has no commit steps.

## Global Constraints

- Strict parity with the web board v1; no new features (no notifications, history, inbox). Web UI untouched.
- The client draws from the daemon's `UiSchema`; it names no status, column or action. The only derived card keys are `repos` and `branch`.
- The UI thread only draws: no network, no filesystem, no process spawning on it. Background threads report through `mpsc` and call a repaint callback.
- Closing the window never stops the daemon without the dialog described in the spec, except when nothing is Queued/Working/Needs you and this client started it.
- The client signals a daemon only through `<state_dir>/daemon.pid`, after checking the pid is a `fleet-daemon` process.
- GUI settings live in `~/.config/fleet-board/gui.toml`; the client ignores unknown keys in `config.toml`.
- Targets: Linux x86_64 on Rocky 8 (glibc 2.28 floor) and Nobara/Fedora (plain native build). `fleet-client` and `fleet-gui` must `cargo check` for `x86_64-pc-windows-gnu` (and optionally `aarch64-apple-darwin` and `x86_64-apple-darwin`); Unix-only code sits in one `cfg(unix)` module with a stub elsewhere, and nothing may depend on Linux-only interfaces such as `/proc`. macOS is not a v0.1 target and is not tested, but must not be made harder.
- Nothing fetched at runtime; no change outside `~/.local` and `~/.config` by `install.sh`; never overwrite an existing `config.toml`.
- Apache-2.0; no company code, names or URLs in the repo.

## Review Focus

Failure modes the spec implies that no task would otherwise test; each is pinned to a task below.

1. The daemon is down, slow or dies mid-session: the window never freezes, shows "reconnecting", and a failed start shows the daemon log tail (Tasks 5, 6, 11, 12).
2. Closing: every dialog branch; a daemon this client did not start, or with agents at risk, is never stopped silently; a pidfile that is stale or names another process is never signalled (Tasks 6, 7).
3. Text with shell metacharacters, `{}`, newlines and non-ASCII in answers and prompts stays literal; a 1 MiB diff or pane does not freeze a frame (Tasks 9, 10).
4. SSE frames split across reads (including inside a multi-byte character), `\r\n` endings, comments, and reconnect with `Last-Event-ID` (Tasks 4, 5).
5. `config.toml` with keys the client does not know, a missing file, and a corrupt `gui.toml` each give a clear message or defaults, never a panic (Task 6).

---

## File Structure

```
crates/fleet-client/   src/{lib,schema,board,sse,http,stream,config,close}.rs  src/daemon_control.rs (cfg unix)
                       tests/{schema,board,sse,config,close}.rs  tests/e2e.rs (real daemon)
crates/fleet-gui/      src/{main,app,theme}.rs  src/widgets/{mod,button,badge,tabs,field}.rs
                       src/components/{mod,card,column,board,repo_filter,spawn_form,preview,close_dialog}.rs
                       tests/*.rs (kittest)
packaging/             install.sh  fleet-board.desktop.in  fleet-board.svg  config.example.toml  test-install.sh
docs/acceptance-gui.md   docs/superpowers/findings/egui-spike.md
```

---

### Task 1: Spike: prove a native window renders here and decide the build path

Exploratory; its deliverables are findings. Stop and report to Antonio if a window cannot open.

**Files:**
- Create: `crates/fleet-gui/` (minimal `main.rs` showing one label and one button), `docs/superpowers/findings/egui-spike.md`

- [ ] **Step 1: Create `crates/fleet-gui`** depending on `eframe = { version = "0.37", default-features = false, features = ["glow", "x11", "wayland", "default_fonts"] }`; add it to the workspace. Show "fleet-board" and a button.
- [ ] **Step 2: Run on the desktop display** (`DISPLAY=:1`) and on `Xvfb :99 -screen 0 1400x800x24`; capture each with `import -window root shot.png`. Record whether the window opens and renders on each, and the GL renderer used.
- [ ] **Step 3: Check linking.** `objdump -T` the binary for its highest `GLIBC_` version; `ldd` it and note whether X11, GL, EGL, xkbcommon are dynamic dependencies or loaded at runtime.
- [ ] **Step 4: Cross-build** with `cargo zigbuild --release -p fleet-gui --target x86_64-unknown-linux-gnu.2.28` and run the result on `:1`. Record whether it works.
- [ ] **Step 5: Check egui_kittest** (`egui_kittest = "0.37"` as a dev-dependency): a test that builds a `Harness`, finds the button by label, clicks it, and asserts a counter changed.
- [ ] **Step 6: Check `cargo check -p fleet-gui --target x86_64-pc-windows-gnu`** (after `rustup target add`). Record the result.
- [ ] **Step 7: Write `egui-spike.md`** answering each. Verify: `docs/superpowers/findings/egui-spike.md` exists, the three screenshots exist, and the kittest test passes. If zig works, `build.sh` uses it for the GUI; if not, Task 13 implements the native fallback.

### Task 2: `fleet-client` schema helpers

**Files:**
- Create: `crates/fleet-client/Cargo.toml`, `src/lib.rs`, `src/schema.rs`; Test: `tests/schema.rs`

**Interfaces:**
- Produces:
  - `pub fn column_for<'a>(schema: &'a UiSchema, a: &Agent) -> Option<&'a ColumnDef>`
  - `pub fn visible_actions<'a>(schema: &'a UiSchema, a: &Agent) -> Vec<&'a ActionDef>` (uses `fleet_api::eval`)
  - `pub struct ActionRequest { pub method: String, pub path: String, pub body: Option<serde_json::Value> }`; `pub fn action_request(action: &ActionDef, agent: &Agent, input: Option<&str>) -> ActionRequest`
  - `pub enum FieldValue { Absent, Flag, Text(String), Number(i64), List(Vec<String>) }`; `pub fn field_value(a: &Agent, key: &str) -> FieldValue` (null, `""`, `false` and unknown keys are `Absent`; `true` is `Flag`; `repos` is the workspace repos; `branch` is the unique branches)
  - `pub fn format_duration(seconds: i64) -> String`

- [ ] **Step 1: Write failing tests** mirroring `web/src/api/schema.test.ts` with the same expectations: `column_for_uses_schema_status_mapping` (Dead is in `review`), `visible_actions_follows_conditions` (working: pause, kill; needs_you: answer, kill; paused review: resume, kill; dead: kill; dead and paused: kill), `action_request_builds_method_path_and_body` (kill is `DELETE /api/agents/abc` with no body; pause is `POST …/pause` with `{"actor":"local"}`; answer carries `{"actor":"local","text":"$(x) \"q\""}` unchanged), `action_request_escapes_the_id` (`a/b?c` becomes `a%2Fb%3Fc`), `field_value_derives_repos_and_branch`, `field_value_absent_for_empty_false_null_and_unknown`, `format_duration_shows_two_largest_units` (`[0,5,65,3700,90000,-4]` → `["0s","5s","1m","1h 1m","1d 1h","0s"]`).
- [ ] **Step 2: Run** `cargo test -p fleet-client --test schema`. Expected: FAIL.
- [ ] **Step 3: Implement** the signatures above; load the default schema in tests from `fleet_api::default_ui_schema()`.
- [ ] **Step 4: Run again.** Expected: PASS.

### Task 3: Board reducer

**Files:** Create `crates/fleet-client/src/board.rs`; Test `tests/board.rs`

**Interfaces:** Produces `pub fn apply(state: &mut Snapshot, e: &Event)`.

- [ ] **Step 1: Write failing tests** mirroring `web/src/api/board.test.ts`: `applies_status_changed_and_removed` (fields, `status_since` from `since`, `blocked_reason`), `appends_created_agents_once`, `ignores_events_at_or_below_the_current_seq`, `unknown_agent_advances_seq_but_changes_nothing`, `applies_workspaces_changed` (sets workspaces and `has_changes` to any workspace's flag; clears it again).
- [ ] **Step 2: Run** `cargo test -p fleet-client --test board`. Expected: FAIL.
- [ ] **Step 3: Implement `apply`.**
- [ ] **Step 4: Run again.** Expected: PASS.

### Task 4: SSE parser

**Files:** Create `crates/fleet-client/src/sse.rs`; Test `tests/sse.rs`

**Interfaces:** Produces `pub struct SseMessage { pub id: Option<String>, pub event: Option<String>, pub data: String }`; `pub struct SseParser`; `SseParser::new() -> Self`; `SseParser::feed(&mut self, chunk: &[u8]) -> Vec<SseMessage>`.

- [ ] **Step 1: Write failing tests:** `parses_id_event_and_data`; `frame_split_across_chunks_at_every_byte_offset_gives_the_same_messages` (loop over all split points, including inside a multi-byte character like `é`); `accepts_crlf_and_lf_endings`; `skips_comment_lines_and_keepalives`; `joins_multiple_data_lines_with_a_newline`; `ignores_a_frame_with_no_fields`; `a_trailing_partial_frame_waits_for_more_bytes`.
- [ ] **Step 2: Run** `cargo test -p fleet-client --test sse`. Expected: FAIL.
- [ ] **Step 3: Implement** a byte buffer parser; decode UTF-8 only on complete lines.
- [ ] **Step 4: Run again.** Expected: PASS.

### Task 5: HTTP client and background streams

**Files:** Create `crates/fleet-client/src/{http,stream}.rs`; Test `crates/fleet-client/tests/e2e.rs` (dev-dependency on `fleet-daemon` for its router; the fake `claude` is `crates/fleet-daemon/tests/bin/claude`)

**Interfaces:**
- Consumes: Tasks 2 to 4.
- Produces:
  - `pub struct FleetError(pub String)` (the daemon's `{"error": …}` text, or `HTTP <status>`, or the transport error)
  - `pub struct FleetClient`; `FleetClient::new(base_url: &str) -> FleetClient`; `snapshot() -> Result<Snapshot, FleetError>`; `ui_schema() -> Result<UiSchema, _>`; `repos() -> Result<Vec<String>, _>`; `spawn(repo: &str, prompt: &str) -> Result<Agent, _>`; `run(action: &ActionDef, agent: &Agent, input: Option<&str>) -> Result<(), _>`; `diff(agent_id: &str) -> Result<DiffReport, _>`; every call has a timeout of 10 s
  - `pub enum StreamMsg { Connected, Disconnected, Snapshot(Snapshot), Event(Event) }`; `pub enum PaneMsg { Text(String), Closed }`
  - `pub struct StreamHandle` (dropping it stops the thread)
  - `pub fn spawn_event_stream(base_url: &str, tx: mpsc::Sender<StreamMsg>, wake: Arc<dyn Fn() + Send + Sync>) -> StreamHandle`; `pub fn spawn_pane_stream(base_url: &str, agent_id: &str, tx: mpsc::Sender<PaneMsg>, wake: Arc<dyn Fn() + Send + Sync>) -> StreamHandle`

- [ ] **Step 1: Write failing tests** in `e2e.rs` against the real router bound to `127.0.0.1:0`: `client_round_trips_against_the_real_daemon` (spawn, snapshot lists it, answer text with `$(x) "q" {a}\n` appears in the pane, pause, resume, diff shows a file written into the worktree, kill removes it); `errors_carry_the_daemons_message` (unknown repo gives a `FleetError` containing it); `event_stream_sends_connected_then_snapshot_then_events`; `stream_reconnects_and_resumes_with_last_event_id` (a hand-written TCP server sends one event, closes, and the test asserts the second request carries `Last-Event-ID` equal to that event's id; the client emits `Disconnected` then `Connected`); `dropping_the_handle_stops_the_thread` (the server sees the connection close within 2 s); `pane_stream_sends_text_then_closed`; `a_daemon_that_never_answers_gives_disconnected_not_a_hang` (the client thread stays responsive to the handle drop).
- [ ] **Step 2: Run** `cargo test -p fleet-client --test e2e`. Expected: FAIL.
- [ ] **Step 3: Implement.** The stream thread reads in chunks into `SseParser`, tracks the last event id, sends `Disconnected` on any error, sleeps with backoff 1 s up to 5 s, and checks its stop flag at least every 500 ms. Every send is followed by `wake()`.
- [ ] **Step 4: Run again.** Expected: PASS.

### Task 6: Config, settings and daemon control; daemon pidfile

**Files:**
- Create: `crates/fleet-client/src/{config,daemon_control}.rs`; Modify: `crates/fleet-daemon/src/main.rs` (pidfile, SIGTERM); Test: `crates/fleet-client/tests/config.rs`, `crates/fleet-client/tests/e2e.rs` (daemon control), `crates/fleet-daemon/tests/main.rs` (pidfile)

**Interfaces:**
- Produces:
  - `pub struct ClientConfig { pub listen: SocketAddr, pub state_dir: PathBuf }`; `pub fn load(path: &Path, home: &Path) -> Result<ClientConfig, ConfigError>` (ignores unknown keys; defaults `127.0.0.1:7878` and `<home>/.local/share/fleet-board`; `~` expanded); `pub fn default_path(home: &Path) -> PathBuf`
  - `pub enum CloseChoice { KeepRunning, StopDaemon }`; `pub struct GuiSettings { pub close_choice: Option<CloseChoice> }`; `GuiSettings::load(path: &Path) -> GuiSettings` (missing or corrupt gives the default, never an error); `GuiSettings::save(&self, path: &Path) -> io::Result<()>`; `pub fn gui_settings_path(home: &Path) -> PathBuf`
  - (`cfg(unix)`) `pub fn is_up(base_url: &str) -> bool`; `pub enum Started { AlreadyRunning, StartedByUs }`; `pub fn ensure_daemon(cfg: &ClientConfig, daemon_bin: &Path, wait: Duration) -> Result<Started, LaunchError>` (`LaunchError`'s message includes the last 20 lines of `<state_dir>/daemon.log`); `pub fn stop_daemon(state_dir: &Path) -> Result<(), StopError>`; `pub fn find_daemon_binary() -> Option<PathBuf>` (next to the current executable, then `PATH`)
  - Daemon: writes `<state_dir>/daemon.pid` (its pid and a newline) after binding, removes it on graceful exit, and exits gracefully on SIGTERM as well as Ctrl-C.

- [ ] **Step 1: Write failing tests.** `config.rs`: `reads_listen_and_state_dir_and_ignores_every_other_key` (a config with `[[repos]]`, `claude_command`, `branch_prefix`), `defaults_when_keys_are_absent`, `missing_file_names_the_path`, `gui_settings_round_trip`, `corrupt_gui_settings_fall_back_to_the_default`. `e2e.rs`: `ensure_daemon_starts_the_real_binary_and_a_second_call_reports_already_running` (uses `CARGO_BIN_EXE_fleet-daemon` from the daemon crate's test via a path helper), `ensure_daemon_failure_includes_the_log_tail` (a binary that prints a reason and exits; the error contains it), `stop_daemon_ends_the_process_and_removes_the_pidfile`, `stop_daemon_refuses_a_pid_that_is_not_a_fleet_daemon` (pidfile names the test process; it is still alive afterwards and the error says why), `a_stale_pidfile_for_a_dead_pid_is_cleaned_not_signalled`. `main.rs` (daemon): `pidfile_holds_the_daemons_pid_while_running_and_is_gone_after_sigterm`.
- [ ] **Step 2: Run** the three test files. Expected: FAIL.
- [ ] **Step 3: Implement.** Identity check without `/proc`: `ps -p <pid> -o comm=` must print a name whose basename is `fleet-daemon`; a pid that `ps` does not know is stale. Also replace the `/proc/self` uid lookup in `Tmux::kill_server_blocking` (`crates/fleet-daemon/src/tmux.rs`) with `libc::getuid()`, so the daemon's test helper is not Linux-only. Start the daemon with `setsid` semantics (new session) and stdout and stderr appended to `daemon.log`. Signal with `libc::kill`.
- [ ] **Step 4: Run again.** Expected: PASS.

### Task 7: Close decision logic

**Files:** Create `crates/fleet-client/src/close.rs`; Test `tests/close.rs`

**Interfaces:** Produces `pub struct Summary { pub running: usize, pub working: usize, pub needs_you: usize }`; `pub enum CloseAction { CloseOnly, StopDaemonAndClose, Ask(Summary) }`; `pub fn close_action(agents: &[Agent], we_started_daemon: bool, remembered: Option<CloseChoice>) -> CloseAction`; `pub fn dialog_text(s: &Summary) -> String`.

- [ ] **Step 1: Write failing tests:** `nothing_at_risk_and_we_started_it_stops_without_asking` (no Queued, Working or Needs you agents); `nothing_at_risk_but_we_did_not_start_it_asks`; `agents_at_risk_ask_unless_a_choice_is_remembered`; `remembered_keep_running_closes_only`; `remembered_stop_daemon_stops_even_with_agents_at_risk` (the user chose this); `running_counts_queued_working_and_needs_you_only` (Review and Dead are not running); `dialog_text_states_the_numbers_and_the_consequence` (contains "3 agents are running", "1 working", "2 waiting for you", and that Needs you requests and finished agents will not be reported while the daemon is stopped); singular wording for one agent.
- [ ] **Step 2: Run** `cargo test -p fleet-client --test close`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run again.** Expected: PASS.

### Task 8: GUI theme and shared widgets

**Files:** Create `crates/fleet-gui/src/{theme.rs,widgets/*.rs}`; Test `crates/fleet-gui/tests/widgets.rs`

**Interfaces:**
- Produces: `theme::apply(ctx: &egui::Context)` (follows the system light/dark preference); `theme::tone_color(tone: Tone, dark: bool) -> Color32` (colours equal to the web CSS variables in `web/src/styles.css`); `widgets::{ButtonVariant { Default, Danger, QuietDanger }, button(ui, label: &str, variant: ButtonVariant) -> Response, toggle(ui, label: &str, pressed: bool) -> Response, badge(ui, label: &str, tooltip: &str), tabs<Id: Copy + PartialEq>(ui, tabs: &[(Id, &str)], selected: Id) -> Option<Id>, field(ui, def: &FieldDef, agent: &Agent, now: i64)}`.

- [ ] **Step 1: Write failing kittest tests** mirroring `web/src/ui/ui.test.tsx`: `button_click_is_reported_and_disabled_buttons_do_not_click`; `toggle_shows_pressed_state`; `badge_shows_text_with_label_as_tooltip`; `tabs_mark_the_selected_tab_and_report_a_click_once`; `field_renders_each_kind` (text, mono, duration `5m`, badge); `field_renders_nothing_for_empty_false_or_unknown`; `field_shows_a_true_flag_as_its_label_and_a_list_as_one_badge_per_entry`; `tone_colors_match_the_web_tokens` (compares against the hex values in `styles.css` for all five tones in light and dark).
- [ ] **Step 2: Run** `cargo test -p fleet-gui --test widgets`. Expected: FAIL.
- [ ] **Step 3: Implement** using `fleet_client::{field_value, format_duration}`; widgets expose their text to accessibility so tests can find them by label.
- [ ] **Step 4: Run again.** Expected: PASS.

### Task 9: Card, column and board

**Files:** Create `crates/fleet-gui/src/components/{card,column,board}.rs`; Test `crates/fleet-gui/tests/board.rs`

**Interfaces:**
- Consumes: Tasks 2, 8.
- Produces: `pub enum Command { Run { action: ActionDef, agent: Agent, input: Option<String> }, Open(String) }`; `pub struct CardState` (answer text per action id, id of the action awaiting a kill confirmation); `components::card(ui, schema: &UiSchema, agent: &Agent, now: i64, state: &mut CardState) -> Vec<Command>`; `components::column(ui, def: &ColumnDef, schema, agents: &[&Agent], now, states: &mut HashMap<String, CardState>) -> Vec<Command>`; `components::board(ui, schema, agents: &[Agent], now, states) -> Vec<Command>`.

- [ ] **Step 1: Write failing kittest tests** mirroring `web/src/components/components.test.tsx` and `web/src/ui/ui.test.tsx`: `board_renders_columns_from_schema_only` (a two-column custom schema draws exactly those; no "Needs you" text); `each_agent_lands_in_the_column_the_schema_maps_its_status_to`; `longest_waiting_card_first` and `column_does_not_reorder_the_callers_list`; `card_shows_project_task_activity_and_time_waiting_for_needs_you` (repo badge, task, activity, `permission` badge, `5m`); `card_shows_only_fields_whose_condition_holds`; `card_answer_returns_the_text_unchanged` (`$(x) "q" {a}` plus a newline and `é`); `empty_answer_is_disabled`; `kill_needs_a_second_click` (first click returns no command; "Confirm Kill" returns `Run`); `clicking_the_card_body_returns_open`; `card_shows_a_changes_badge_only_when_the_agent_has_changes`; `one_badge_per_workspace_repo`.
- [ ] **Step 2: Run** `cargo test -p fleet-gui --test board`. Expected: FAIL.
- [ ] **Step 3: Implement.** Each column scrolls on its own; a card has a tone-coloured left edge from `tone_color`.
- [ ] **Step 4: Run again.** Expected: PASS.

### Task 10: Repo filter, spawn form, preview pane

**Files:** Create `crates/fleet-gui/src/components/{repo_filter,spawn_form,preview}.rs`; Test `crates/fleet-gui/tests/panels.rs`

**Interfaces:**
- Produces: `repo_filter(ui, repos: &[String], selected: &mut Vec<String>) -> bool` (true when changed; click order preserved); `pub struct SpawnState { pub repo: String, pub prompt: String, pub busy: bool, pub error: Option<String> }`; `spawn_form(ui, repos: &[String], st: &mut SpawnState) -> Option<(String, String)>`; `pub enum PreviewTab { Terminal, Diff }`; `pub struct PreviewState { pub agent_id: String, pub title: String, pub tab: PreviewTab, pub text: String, pub closed: bool, pub diff: DiffView }`; `pub enum DiffView { NotLoaded, Loading, Loaded(DiffReport), Failed(String) }`; `preview_pane(ui, st: &mut PreviewState) -> PreviewAction` where `PreviewAction { None, LoadDiff, Close }`.

- [ ] **Step 1: Write failing kittest tests** mirroring the web's: `repo_filter_toggles_repos_in_click_order`; `spawn_is_disabled_until_the_prompt_has_text`; `spawn_returns_repo_and_prompt_unchanged` (metacharacters, newline, `é`); `spawn_error_is_shown_and_the_prompt_kept`; `preview_terminal_tab_is_read_only` (no text input exists); `preview_says_session_ended_when_closed`; `diff_tab_requests_a_load_and_shows_each_workspace` (a repo with no changes shows "No changes"; a truncated one shows the cut note); `diff_failure_shows_an_alert_and_refresh_requests_another_load`; `a_one_mebibyte_pane_text_renders_a_frame_within_100ms` (virtualized or clipped rendering; measures `Harness::run` time).
- [ ] **Step 2: Run** `cargo test -p fleet-gui --test panels`. Expected: FAIL.
- [ ] **Step 3: Implement.** Use a monospace scroll area that lays out only visible rows for the pane and the diff; colour `+` and `-` diff lines.
- [ ] **Step 4: Run again.** Expected: PASS.

### Task 11: App state and wiring

**Files:** Create `crates/fleet-gui/src/app.rs`; Test `crates/fleet-gui/tests/app.rs`

**Interfaces:**
- Consumes: Tasks 5, 9, 10.
- Produces: `pub struct FleetApp`; `FleetApp::new(client: FleetClient, ctx: egui::Context) -> FleetApp` (starts the event stream and loads schema and repos on background threads); `FleetApp::handle_stream(&mut self, m: StreamMsg)`; `FleetApp::handle_result(&mut self, r: AppResult)`; `FleetApp::ui(&mut self, ui: &mut egui::Ui)`; `pub enum AppResult { Schema(UiSchema), Repos(Vec<String>), Action(Result<(), String>), Spawn(Result<Agent, String>), Diff(String, Result<DiffReport, String>) }`. Every network call runs on its own thread and returns through a channel.

- [ ] **Step 1: Write failing kittest tests** mirroring `web/src/App.test.tsx`: `a_card_moves_when_a_status_event_arrives`; `repo_filter_shows_agents_touching_any_selected_repo` (an agent with two workspaces shows when either is selected); `running_an_action_goes_through_the_client_and_shows_a_failure`; `clicking_a_card_opens_a_read_only_preview_with_a_diff_tab`; `connection_indicator_follows_connected_and_disconnected` ("live" and "reconnecting…"); `a_snapshot_replaces_the_board`; `the_ui_does_not_block_while_the_daemon_is_unreachable` (a `FleetClient` pointed at a closed port: ten frames complete within 200 ms in total); `schema_load_failure_shows_the_error_and_a_retry`.
- [ ] **Step 2: Run** `cargo test -p fleet-gui --test app`. Expected: FAIL.
- [ ] **Step 3: Implement.** Layout as the web board: top bar, spawn form, board, preview side panel.
- [ ] **Step 4: Run again.** Expected: PASS.

### Task 12: Close dialog and `main`

**Files:** Create `crates/fleet-gui/src/components/close_dialog.rs`; Modify `crates/fleet-gui/src/main.rs`; Test `crates/fleet-gui/tests/close_dialog.rs`

**Interfaces:**
- Consumes: Tasks 6, 7, 11.
- Produces: `close_dialog(ui, summary: &Summary, remember: &mut bool) -> Option<CloseChoice>`; `main` accepts `--config <path>` and `--version`; startup order: load config, `ensure_daemon`, run the window; a launch failure draws an error screen with the reason, the log path and a Retry button; on a close request the window calls `close_action`, cancels the close while the dialog is open, applies the choice (`stop_daemon` for `StopDaemon`), saves "remember" to `gui.toml`, then closes.

- [ ] **Step 1: Write failing tests:** `dialog_shows_the_numbers_both_choices_and_a_remember_box` (default focus is Keep running); `keep_running_returns_keep_running_and_stop_returns_stop_daemon`; `remember_is_reported_with_the_choice`; `closing_with_nothing_at_risk_and_a_daemon_we_started_stops_it_silently`; `closing_a_daemon_we_did_not_start_asks`; `a_remembered_choice_skips_the_dialog_and_is_changeable_from_settings`; `version_flag_prints_the_workspace_version` (runs the binary).
- [ ] **Step 2: Run** `cargo test -p fleet-gui --test close_dialog`. Expected: FAIL.
- [ ] **Step 3: Implement.** Settings: a small control in the top bar that lists the remembered choice and clears it.
- [ ] **Step 4: Run again.** Expected: PASS.

### Task 13: Build and install

**Files:** Modify `build.sh`; Create `packaging/{install.sh,fleet-board.desktop.in,fleet-board.svg,config.example.toml,test-install.sh}`

**Interfaces:** `build.sh` builds `fleet-gui` (x86_64) into `dist/` (zig build at glibc 2.28 per Task 1's finding, else native `cargo build --release` with a message naming which it used), copies `packaging/` into `dist/`, and with `FLEET_CHECK_WINDOWS=1` runs `cargo check -p fleet-client -p fleet-gui --target x86_64-pc-windows-gnu`; with `FLEET_CHECK_MACOS=1` the same for `aarch64-apple-darwin` and `x86_64-apple-darwin` (a compile check only, no SDK needed for pure-Rust crates; a failure there is reported, not fatal). `install.sh` takes `--prefix` (default `~/.local`) and honours `HOME`.

- [ ] **Step 1: Write `test-install.sh`** (plain bash, no framework) running `install.sh` with a temporary `HOME` and fake `tmux` and `claude` on `PATH`: `installs_both_binaries_and_the_launcher_and_icon`; `launcher_exec_runs_through_a_login_shell` (the `.desktop` `Exec` starts with `bash -lc`); `never_overwrites_an_existing_config`; `writes_the_config_template_when_absent`; `fails_with_the_fix_when_tmux_is_older_than_3_2` (fake `tmux -V` prints `tmux 2.7`); `fails_with_a_message_when_claude_is_missing`; `prints_the_one_time_trust_step`; `touches_nothing_outside_home_local_and_config`.
- [ ] **Step 2: Run** `bash packaging/test-install.sh`. Expected: FAIL.
- [ ] **Step 3: Implement `install.sh`, the desktop template, the icon and the config template**; update `build.sh`.
- [ ] **Step 4: Run** `bash packaging/test-install.sh`, `./build.sh`, and (once) `FLEET_CHECK_WINDOWS=1 ./build.sh`. Expected: PASS; `dist/` holds `fleet-daemon-*`, `fleet-gui-x86_64-linux` and `packaging/`; the Windows check compiles.

### Task 14: Acceptance and the local tag

Not code. Claude runs it with Antonio's `claude` login on throwaway repos, per the spec.

**Files:** Create `docs/acceptance-gui.md` (the web checklist adapted to the egui app, plus the close dialog and the install steps); Create `docs/superpowers/findings/egui-acceptance.md` (results and screenshot path)

- [ ] **Step 1: Run `install.sh` into a temporary prefix and `HOME`**; start `fleet-gui` on `Xvfb :99`; confirm it started the daemon (`daemon.pid` exists; `daemon.log` shows it listening).
- [ ] **Step 2: Drive real agents through the daemon API** exactly as the web acceptance run did (spawn in two repos, Needs you, answer, Review with changes, pause and resume, kill cleanup, daemon restart with a blocked agent), and capture a screenshot of the window after each state change; verify each card appears in the right column.
- [ ] **Step 3: Verify the close flow** with the UI test harness results from Task 12, and by closing the window with an agent Working: the daemon keeps running (`daemon.pid` still valid) unless the dialog's Stop was chosen.
- [ ] **Step 4: Record results** in `egui-acceptance.md`. Expected: every box ticked, or each failure listed as a bug and fixed before Step 5.
- [ ] **Step 5: Run the whole suite:** `cargo test --workspace`, `pnpm --dir web test`, `bash packaging/test-install.sh`. Expected: all pass.
- [ ] **Step 6: Create the local tag** `v0.1.0` on the commit Antonio has made, only if he has committed the work; otherwise report that the tag is pending his commit. Do not push; publishing needs his go-ahead.

---

## Self-review

- **Spec coverage:** parity items (Tasks 8 to 11); schema-driven rendering and derived keys (2, 8, 9); background threads (5, 11); daemon survives the window and the close dialog (6, 7, 12); client starts the daemon with log tail (6, 12); pidfile identity (6); `gui.toml` and unknown-key tolerance (6); install and launcher through a login shell, prerequisite check (13); native fallback and Windows check (1, 13); Nobara build (plain `cargo build`, Task 1 and 13); tests (all tasks); acceptance on the real `claude` through Xvfb and a local tag (14).
- **Type consistency:** `StreamMsg`, `PaneMsg`, `FleetClient`, `CloseChoice`, `Summary`, `CloseAction`, `Command`, `CardState`, `PreviewState` and `DiffView` are defined once and reused by name later.
- **Left to the builder, per spec:** egui feature flags beyond those named, fonts and spacing, the thread layout beyond the rules above, icon artwork.
