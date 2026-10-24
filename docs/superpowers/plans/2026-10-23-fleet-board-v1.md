# fleet-board v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A local daemon plus web board that runs Claude Code agents in tmux and git worktrees across repos and shows accurate, hook-driven status on a live board with simple fleet controls.

**Architecture:** A Rust daemon (`fleet-daemon`) owns all state in SQLite and exposes one contract (`fleet-api` crate: HTTP JSON commands, an SSE event stream with sequence numbers, a read-only terminal stream). Each agent is a tmux session running the real `claude` in its own git worktree, started with a generated `--settings` file whose hooks report events back to the daemon. A React/TypeScript UI is a pure client of that contract, embedded in the daemon binary.

**Tech Stack:** Rust (Axum, Tokio, sqlx + SQLite, utoipa, ts-rs, rust-embed), tmux 3.x, git worktrees, React + TypeScript + Vite (the read-only preview renders plain text, so xterm.js is not needed until the preview becomes interactive).

**Spec:** `docs/superpowers/specs/2026-10-23-fleet-board-design.md`. Deferred work is in `TODO.md`. Out of v1: login page, agent messaging and the Talking column, bulk approve, orchestrator, egui, multi-user, release pipeline.

## Global Constraints

- Rust daemon, cross-compiled to Linux x86_64 and Linux aarch64. No Windows support (tmux is POSIX-only).
- Axum on Tokio; state in SQLite; events over SSE; OpenAPI via utoipa; TypeScript types generated from the Rust crate via ts-rs. The UI is React + TypeScript + Vite, built into the daemon binary and served by it. Nothing is fetched at runtime.
- Binds `127.0.0.1` only, no auth in v1. The daemon refuses to start on a non-loopback address.
- tmux 3.2 or newer required (`new-session -e` sets the per-agent token; developed against 3.8 in `~/.local/bin`); 2.7 is unsupported.
- One tmux session per agent running the real `claude` CLI; one git worktree and branch per agent. Branch name `<prefix>/<task-slug>` from the repo's current HEAD, prefix configurable.
- Repos come from `~/.config/fleet-board/config.toml` (a list of paths); no directory scanning.
- Status comes from Claude Code hooks. Never infer status by scraping pane text. tmux is used only for the preview, input and liveness.
- Never read, copy or store Claude login tokens. v1 uses the `claude` login already in `~/.claude`.
- Every command carries `actor` and `project`. Value in v1: actor `"local"`.
- Every agent carries an `owner` (the local user in v1) so team mode can be added without changing the contract (`docs/concept.md`).
- A Needs you card says why it is blocked (`blocked_reason`, Antonio 2026-10-23; mapping under "Verified facts"). The reason comes from the hook event or the startup timer, never from pane text.
- Terminal preview is read-only. Answering types into the pane. Pause sends an interrupt (Escape) and marks the card paused; resume sends a short continue prompt.
- Review means the agent stopped and its branch has changes; v1 only shows the diff (no PR or merge action).
- Columns: Queued, Working, Needs you, Talking (empty in v1), Review.
- The UI is data-driven (Antonio, 2026-10-23): columns, card fields and per-status actions are defined once in the daemon as a `UiSchema` (`fleet-api`, served at `GET /api/ui-schema`); React components render generically from it and hard-code no status, column or action names.
- License Apache-2.0. No company code, prompts, repo names or internal URLs in the repo.

## Verified facts (hooks docs, 2026-10-23)

Events used and their status mapping, implemented in Task 3:

| Hook event | Status |
|---|---|
| `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `SubagentStart`, `PermissionDenied` | Working (activity from `tool_name`) |
| `PermissionRequest` (immediate), `Notification` with `permission_prompt`, `elicitation_dialog`, `elicitation_url_dialog`, `agent_needs_input` | Needs you |
| `Stop`, `Notification` `idle_prompt` | Review |
| `StopFailure` | Needs you (blocked reason Error) |
| `SessionEnd`, or tmux session gone | Dead (shown in Review with a badge) |

Blocked reason on Needs you cards: `PermissionRequest` and `permission_prompt` give Permission; `elicitation_*` and `agent_needs_input` give Question; `StopFailure` gives Error; the 20-second no-`SessionStart` rule gives Startup. Every other status has none.

Common input fields: `session_id`, `cwd`, `hook_event_name`, `tool_name`, `tool_input`. Handlers can be `command` or `http` (POST of the event JSON). `claude --settings <file>` is supported by the installed 2.1.309.

## Review Focus

Failure modes the spec implies that no task would otherwise test; each is pinned to a task below.

1. A prompt or answer containing shell metacharacters, quotes, newlines or a leading `-` is delivered literally and never executed or parsed as a flag (Task 7, Task 9).
2. Spawning when the repo path has spaces, is not a git repo, or fails halfway leaves no orphan tmux session, worktree or branch (Task 6, Task 9).
3. Two agents with the same task slug in the same repo get distinct branches and worktrees (Task 6).
4. Hook events for an unknown agent, with a wrong token, duplicated, or out of order never corrupt state or revive a Dead agent (Task 3, Task 8).
5. A daemon restart while agents run, or a tmux session killed behind its back, ends with every card in a truthful state, never stuck on Working (Task 9). The first-run "trust this folder" prompt, which fires no hook, must not leave a card silently on Queued (Task 1, Task 9).
6. An SSE client that reconnects after missing events gets a consistent board via `Last-Event-ID` or a fresh snapshot (Task 10).

---

## File Structure

```
Cargo.toml                          workspace
crates/fleet-api/                   contract types only; no I/O
  src/lib.rs, src/status.rs, src/model.rs, src/event.rs, src/requests.rs
  tests/contract.rs                 OpenAPI + TS export snapshot tests
crates/fleet-daemon/
  src/main.rs                       CLI entry: load config, bind, serve
  src/config.rs                     config.toml loading, loopback check
  src/store.rs                      SQLite: agents, event log (seq)
  src/status.rs                     pure hook-event -> status reducer
  src/git.rs                        worktree + branch management
  src/tmux.rs                       tmux wrapper (own socket-name aware)
  src/hooks_cfg.rs                  generates per-agent --settings JSON
  src/agents.rs                     service: spawn/pause/resume/kill/answer/reconcile
  src/http.rs                       Axum routes, SSE, terminal stream, embedded UI
  tests/*.rs                        per-module integration tests
web/                                Vite + React + TS UI (pure client)
  src/api/generated/                ts-rs output (committed, regenerated by script)
  src/api/client.ts, src/api/events.ts
  src/components/{Board,Column,Card,ProjectFilter,SpawnForm,PreviewPane}.tsx
fixtures/hooks/*.json               real hook payloads captured in Task 1
docs/superpowers/findings/hooks-spike.md
```

---

### Task 1: Phase 0 spike — prove the status model on the real CLI

Exploratory by design; its deliverables are findings plus fixtures, and later tasks must be adjusted if findings contradict this plan.

**Files:**
- Create: `docs/superpowers/findings/hooks-spike.md`, `fixtures/hooks/<event>-<n>.json`, `scripts/spike/receiver.py` (throwaway; delete once the daemon receives hooks)

**Interfaces:**
- Produces: captured hook JSON per event type (used as fixtures by Task 3 and Task 8) and answers to the four questions below.

- [ ] **Step 1: Run a loopback receiver** on `127.0.0.1` that appends every POSTed JSON body to `fixtures/hooks/raw.jsonl` and returns `{}`.
- [ ] **Step 2: Write a settings file** with `http` hooks (URL = the receiver) for `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PermissionRequest`, `PostToolUse`, `PostToolUseFailure`, `Notification`, `Stop`, `StopFailure`, `SessionEnd`.
- [ ] **Step 3: Start an interactive agent** in a throwaway git worktree: `tmux new-session -d -s spike -c <worktree> claude --settings <file>`; type a prompt that edits a file and runs a command needing permission; approve it.
- [ ] **Step 4: Record in `hooks-spike.md`**: (a) did every hook fire in an interactive tmux session with `--settings`; (b) did http hooks reach loopback, and can the URL carry a per-agent token in its path; (c) what happens on a fresh worktree before any hook (the "trust this folder" prompt) — which hook, if any, fires, and whether the prompt can be avoided without touching `~/.claude` (try `--settings` and `--permission-mode`); (d) the exact event order for prompt → tool → permission → approve → stop.
- [ ] **Step 5: Split `raw.jsonl` into `fixtures/hooks/<event>-<n>.json`.**
- [ ] **Step 6: Verify:** `ls fixtures/hooks | wc -l` is at least 10 and `hooks-spike.md` answers (a)–(d). If (a) or (b) is "no", stop and bring the finding to Antonio before Task 2.

### Task 2: Workspace and `fleet-api` contract types

**Files:**
- Create: `Cargo.toml`, `crates/fleet-api/Cargo.toml`, `crates/fleet-api/src/{lib,status,model,event,requests}.rs`
- Test: `crates/fleet-api/tests/contract.rs`

**Interfaces:**
- Produces (all `Serialize, Deserialize, ToSchema, TS`, `Clone, Debug, PartialEq`):
  - `enum Status { Queued, Working, NeedsYou, Talking, Review, Dead }`
  - `struct AgentId(pub String)`; `struct Actor(pub String)`
  - `struct Agent { id: AgentId, project: String, task: String, branch: String, worktree: String, tmux_session: String, status: Status, activity: String, status_since: i64, has_changes: bool, paused: bool, owner: String, blocked_reason: Option<BlockedReason> }`; `enum BlockedReason { Permission, Question, Error, Startup }` (set only while `NeedsYou`; `owner` is the local user until a team hub exists, per `docs/concept.md`)
  - `enum EventKind { AgentCreated { agent: Agent }, StatusChanged { id: AgentId, status: Status, activity: String, since: i64, has_changes: bool, paused: bool, blocked_reason: Option<BlockedReason> }, AgentRemoved { id: AgentId } }` and `struct Event { seq: u64, kind: EventKind }`
  - `struct Snapshot { seq: u64, agents: Vec<Agent> }`
  - `struct SpawnRequest { actor: Actor, project: String, prompt: String, task_slug: Option<String> }`, `struct AnswerRequest { actor: Actor, text: String }`, `struct ControlRequest { actor: Actor }`
  - `struct UiSchema { columns: Vec<ColumnDef>, card_fields: Vec<FieldDef>, actions: Vec<ActionDef> }`; `struct ColumnDef { id: String, title: String, statuses: Vec<Status>, empty_hint: String, tone: Tone }`; `enum Tone { Neutral, Active, Attention, Done, Muted }`; `struct FieldDef { key: String, label: String, kind: FieldKind, show_when: Condition }`; `enum FieldKind { Text, Duration, Badge, Mono }`; `struct ActionDef { id: String, label: String, method: String, path: String, when: Condition, input: Option<String>, destructive: bool }` (`path` uses `{id}`; `input` is the request-body field the UI must collect, e.g. `"text"`); `enum Condition { Always, StatusIn { statuses: Vec<Status> }, Paused { value: bool }, HasChanges { value: bool }, All { conditions: Vec<Condition> } }` (internally tagged by `type`)
  - `fn default_ui_schema() -> UiSchema` (the five columns, the card fields project/task/activity/time-in-status, and the Answer, Pause, Resume, Kill actions) and `fn eval(c: &Condition, a: &Agent) -> bool`
  - `fn openapi() -> utoipa::openapi::OpenApi` in `lib.rs`

- [ ] **Step 1: Write failing tests** in `contract.rs`: `status_serializes_snake_case` (`NeedsYou` ⇒ `"needs_you"`), `event_round_trips_json`, `openapi_lists_all_routes` (paths `/api/agents`, `/api/agents/{id}/answer`, `/api/agents/{id}/pause`, `/api/agents/{id}/resume`, `/api/agents/{id}`, `/api/events`, `/api/agents/{id}/terminal`), `default_ui_schema_covers_every_status_in_exactly_one_column` (each `Status` except `Talking`'s empty column appears in one `ColumnDef`; `Talking` maps to the Talking column), `eval_conditions` (`All`, `StatusIn`, `Paused`), `ts_bindings_export` (running `cargo test -p fleet-api` writes `web/src/api/generated/*.ts`).
- [ ] **Step 2: Run** `cargo test -p fleet-api`. Expected: FAIL (types missing).
- [ ] **Step 3: Implement the types above** with `#[serde(rename_all = "snake_case")]` and `#[ts(export, export_to = "../../web/src/api/generated/")]`; `openapi()` assembles the routes listed in the test.
- [ ] **Step 4: Run** `cargo test -p fleet-api`. Expected: PASS and `ls web/src/api/generated` shows `Agent.ts`, `Status.ts`, `Event.ts`.

### Task 3: Status reducer

**Files:**
- Create: `crates/fleet-daemon/src/status.rs`
- Test: `crates/fleet-daemon/tests/status.rs`

**Interfaces:**
- Consumes: `Status` from Task 2; fixtures from Task 1.
- Produces:
  - `enum HookEvent { SessionStart, UserPromptSubmit, PreToolUse { tool: String }, PermissionRequest { tool: String }, PostToolUse, PostToolUseFailure, PermissionDenied, SubagentStart, Notification { kind: NotificationKind }, Stop, StopFailure, SessionEnd, Other(String) }`
  - `enum NotificationKind { PermissionPrompt, IdlePrompt, Elicitation, AgentNeedsInput, Other(String) }`
  - `fn parse_hook(body: &serde_json::Value) -> Result<HookEvent, ParseError>` (reads `hook_event_name`, `tool_name`, `notification_type`)
  - `struct AgentState { status: Status, activity: String, paused: bool }`
  - `fn reduce(prev: &AgentState, ev: &HookEvent) -> Option<AgentState>` (`None` = no change)

- [ ] **Step 1: Write failing tests**: one table-driven test `maps_every_event_in_the_spec_table` asserting the mapping in "Verified facts" (e.g. `PermissionRequest{tool:"Bash"}` ⇒ `NeedsYou`, activity `"wants to run Bash"`; `Stop` ⇒ `Review`); `permission_resolved_by_post_tool_use` (`NeedsYou` + `PostToolUse` ⇒ `Working`); `dead_never_revives` (`Dead` + any event ⇒ `None`); `duplicate_event_is_no_change`; `parses_every_fixture` (each `fixtures/hooks/*.json` parses to something other than `Err`); `unknown_event_is_other_and_ignored`.
- [ ] **Step 2: Run** `cargo test -p fleet-daemon --test status`. Expected: FAIL.
- [ ] **Step 3: Implement `parse_hook` and `reduce`** as pure functions, no I/O. A paused agent stays paused until `UserPromptSubmit`.
- [ ] **Step 4: Run again.** Expected: PASS.

### Task 4: SQLite store and event log

**Files:**
- Create: `crates/fleet-daemon/src/store.rs`
- Test: `crates/fleet-daemon/tests/store.rs`

**Interfaces:**
- Consumes: `Agent`, `Event`, `EventKind`, `Snapshot`.
- Produces: `struct Store` with
  - `async fn open(path: &Path) -> Result<Store>` (runs migrations; `:memory:` allowed)
  - `async fn insert_agent(&self, a: &Agent, token: &str) -> Result<Event>`
  - `async fn update_agent(&self, id: &AgentId, f: impl FnOnce(&mut Agent)) -> Result<Option<Event>>` (appends a `StatusChanged` event only if fields changed)
  - `async fn remove_agent(&self, id: &AgentId) -> Result<Event>`
  - `async fn snapshot(&self) -> Result<Snapshot>`; `async fn events_since(&self, seq: u64) -> Result<Option<Vec<Event>>>` (`None` when `seq` is older than the retained log)
  - `async fn agent_token(&self, id: &AgentId) -> Result<Option<String>>`

- [ ] **Step 1: Write failing tests**: `seq_is_monotonic_across_writes`; `unchanged_update_emits_no_event`; `events_since_returns_gap_free_tail`; `events_since_old_seq_returns_none_after_trim` (log retention capped at 10 000 events); `snapshot_seq_matches_last_event`.
- [ ] **Step 2: Run** `cargo test -p fleet-daemon --test store`. Expected: FAIL.
- [ ] **Step 3: Implement with sqlx and SQL migrations** in `crates/fleet-daemon/migrations/`; the event row insert and the agent write share one transaction.
- [ ] **Step 4: Run again.** Expected: PASS.

### Task 5: Config and loopback guard

**Files:**
- Create: `crates/fleet-daemon/src/config.rs`
- Test: `crates/fleet-daemon/tests/config.rs`

**Interfaces:**
- Produces: `struct Config { listen: SocketAddr, state_dir: PathBuf, branch_prefix: String, repos: Vec<Repo> }`, `struct Repo { name: String, path: PathBuf }`, `fn load(path: &Path) -> Result<Config, ConfigError>`, `fn default_path() -> PathBuf` (`~/.config/fleet-board/config.toml`). Defaults: `listen = 127.0.0.1:7878`, `branch_prefix = "fleet"`, `state_dir = ~/.local/share/fleet-board`.

- [ ] **Step 1: Write failing tests**: `loads_repos_and_defaults`; `rejects_non_loopback_listen` (`0.0.0.0:7878` and `192.168.1.5:7878` ⇒ `ConfigError::NonLoopback`); `rejects_duplicate_repo_names`; `rejects_repo_path_that_is_not_a_directory`.
- [ ] **Step 2: Run** `cargo test -p fleet-daemon --test config`. Expected: FAIL.
- [ ] **Step 3: Implement** with serde + toml; `~` expanded.
- [ ] **Step 4: Run again.** Expected: PASS.

### Task 6: Git worktree manager

**Files:**
- Create: `crates/fleet-daemon/src/git.rs`
- Test: `crates/fleet-daemon/tests/git.rs`

**Interfaces:**
- Produces: `struct Worktree { path: PathBuf, branch: String }`; `async fn create(repo: &Path, prefix: &str, slug: &str, root: &Path) -> Result<Worktree>`; `async fn remove(repo: &Path, wt: &Worktree) -> Result<()>`; `async fn has_changes(wt: &Path) -> Result<bool>` (commits ahead of the base branch or a dirty tree); `fn slugify(prompt: &str) -> String`. Shell out to `git` with argv, never through a shell.

- [ ] **Step 1: Write failing tests** against a temp repo: `creates_branch_from_head_in_new_worktree`; `same_slug_twice_gets_distinct_branch_and_path` (second is `<slug>-2`); `repo_path_with_spaces_works`; `not_a_git_repo_is_typed_error` and leaves nothing on disk; `remove_deletes_worktree_and_branch`; `has_changes_false_on_fresh_worktree_true_after_commit`; `slugify_strips_shell_metacharacters_and_caps_length` (`"fix; rm -rf /"` ⇒ `fix-rm-rf`).
- [ ] **Step 2: Run** `cargo test -p fleet-daemon --test git`. Expected: FAIL.
- [ ] **Step 3: Implement.** On any failure after `git worktree add`, remove the worktree and delete the branch before returning the error.
- [ ] **Step 4: Run again.** Expected: PASS.

### Task 7: tmux wrapper

**Files:**
- Create: `crates/fleet-daemon/src/tmux.rs`
- Test: `crates/fleet-daemon/tests/tmux.rs`

**Interfaces:**
- Produces: `struct Tmux { socket: Option<String> }` (`Some` names a private server via `-L`, used by tests) with `async fn new_session(&self, name: &str, cwd: &Path, argv: &[String]) -> Result<()>` (argv executed directly, no shell); `async fn send_literal(&self, name: &str, text: &str) -> Result<()>` (typed with `send-keys -l`, then Enter as a separate key); `async fn send_key(&self, name: &str, key: &str) -> Result<()>`; `async fn capture(&self, name: &str) -> Result<String>`; `async fn has_session(&self, name: &str) -> Result<bool>`; `async fn list_sessions(&self, prefix: &str) -> Result<Vec<String>>`; `async fn kill(&self, name: &str) -> Result<()>`; `async fn version(&self) -> Result<(u32, u32)>`.

- [ ] **Step 1: Write failing tests** on a private socket: `rejects_tmux_older_than_3` (via an injectable version check); `session_lifecycle`; `send_literal_delivers_metacharacters_verbatim` (send `$(touch /tmp/x); 'a" -n\n b` into `cat`, assert captured text equals it and `/tmp/x` does not exist); `leading_dash_text_is_not_a_flag`; `list_sessions_filters_by_prefix`.
- [ ] **Step 2: Run** `cargo test -p fleet-daemon --test tmux`. Expected: FAIL.
- [ ] **Step 3: Implement** by spawning `tmux` with argv and `--` before any user text.
- [ ] **Step 4: Run again.** Expected: PASS.

### Task 8: Hook settings generator and hook ingress

**Files:**
- Create: `crates/fleet-daemon/src/hooks_cfg.rs`; hook route in `crates/fleet-daemon/src/http.rs` (route only; the rest of `http.rs` is Task 10)
- Test: `crates/fleet-daemon/tests/hooks.rs`

**Interfaces:**
- Consumes: `parse_hook`, `reduce`, `Store`.
- Produces: `fn write_settings(dir: &Path, base_url: &str, agent: &AgentId) -> Result<PathBuf>` writing a settings JSON whose hooks (events from the table in "Verified facts") POST to `{base_url}/internal/hooks/{agent}` with header `X-Fleet-Token: $FLEET_TOKEN` (http hooks list `FLEET_TOKEN` in `allowedEnvVars`); `SessionStart` and `SessionEnd` are `command` hooks that pipe stdin to the same URL with `curl` and the same header, because those two events do not support http handlers; `async fn ingest(store: &Store, agent: &AgentId, token_header: Option<&str>, body: serde_json::Value) -> Result<IngestOutcome>` with `enum IngestOutcome { Applied(Event), Ignored }` and errors `UnknownAgent`, `BadToken`. Task 1 confirmed this design (`docs/superpowers/findings/hooks-spike.md`).

- [ ] **Step 1: Write failing tests**: `settings_file_contains_a_hook_for_each_mapped_event`; `ingest_with_wrong_token_is_rejected_and_state_unchanged`; `ingest_for_unknown_agent_is_rejected`; `ingest_replays_fixtures_to_expected_final_status` (replay Task 1's recorded order ⇒ `Review`); `duplicate_and_out_of_order_events_do_not_revive_dead`.
- [ ] **Step 2: Run** `cargo test -p fleet-daemon --test hooks`. Expected: FAIL.
- [ ] **Step 3: Implement `write_settings` and `ingest`.** The route replies `200 {}` quickly; apply the reducer under the store's transaction.
- [ ] **Step 4: Run again.** Expected: PASS.

### Task 9: Agent service (spawn, pause, resume, kill, answer, reconcile)

**Files:**
- Create: `crates/fleet-daemon/src/agents.rs`
- Test: `crates/fleet-daemon/tests/agents.rs` (uses a fake `claude` script in `tests/bin/claude` that blocks on stdin, and a private tmux socket)

**Interfaces:**
- Consumes: `Config`, `Store`, `git::*`, `Tmux`, `hooks_cfg::write_settings`, `reduce`.
- Produces: `struct AgentService` with `async fn spawn(&self, req: SpawnRequest) -> Result<Agent, SpawnError>`; `async fn answer(&self, id: &AgentId, text: &str) -> Result<()>`; `async fn pause(&self, id: &AgentId) -> Result<()>`; `async fn resume(&self, id: &AgentId) -> Result<()>`; `async fn kill(&self, id: &AgentId) -> Result<()>` (kills tmux, removes worktree and branch); `async fn reconcile(&self) -> Result<()>` (run at startup and every 5 s); `enum SpawnError { UnknownProject, Git(..), Tmux(..) }`. Session names are `fleet-<first 8 of id>`; the token reaches the process as `FLEET_TOKEN` via `tmux new-session -e`, and worktrees live under `<state_dir>/worktrees`, a root the user trusts once by running `claude` there (trust is inherited by child directories). The daemon never sets `--permission-mode`. The prompt is sent via `send_literal` after the session starts, not as a shell argument. An agent with no `SessionStart` event 20 s after spawn is marked `NeedsYou` with `blocked_reason = Startup` and activity `"waiting at startup prompt"` (derived from tmux liveness and timing, not text).

- [ ] **Step 1: Write failing tests**: `spawn_creates_worktree_session_and_queued_card`; `spawn_unknown_project_leaves_nothing_behind`; `spawn_failure_after_worktree_cleans_up_worktree_and_branch` (fake claude path invalid ⇒ no worktree, no branch, no session); `prompt_with_shell_metacharacters_reaches_the_pane_verbatim`; `pause_sends_escape_and_sets_paused`; `resume_clears_paused_and_sends_continue`; `kill_removes_session_worktree_and_card`; `reconcile_marks_dead_when_session_vanished`; `reconcile_after_restart_adopts_live_sessions` (new `AgentService` over the same store keeps live agents and kills none); `no_session_start_event_for_20s_marks_needs_you` (inject a fake clock; asserts `blocked_reason == Some(Startup)`).
- [ ] **Step 2: Run** `cargo test -p fleet-daemon --test agents`. Expected: FAIL.
- [ ] **Step 3: Implement the service.** `spawn` order: validate project → worktree → settings file → tmux session → send prompt → insert agent; undo in reverse order on any failure.
- [ ] **Step 4: Run again.** Expected: PASS.

### Task 10: HTTP API, SSE events and terminal stream

**Files:**
- Modify: `crates/fleet-daemon/src/http.rs`; Create: `crates/fleet-daemon/src/main.rs`
- Test: `crates/fleet-daemon/tests/http.rs`

**Interfaces:**
- Consumes: `AgentService`, `Store`, `fleet_api::openapi`.
- Produces: routes `GET /api/agents` (→ `Snapshot`), `POST /api/agents` (`SpawnRequest` → `Agent`), `POST /api/agents/{id}/answer|pause|resume`, `DELETE /api/agents/{id}`, `GET /api/events` (SSE; event `id:` = `seq`; honors `Last-Event-ID`; if the gap cannot be served it sends one `snapshot` event with the full `Snapshot` first), `GET /api/agents/{id}/terminal` (SSE of pane text polled every 500 ms, read-only), `GET /api/ui-schema` (→ `UiSchema`), `GET /api/openapi.json`, `POST /internal/hooks/{id}` with an `X-Fleet-Token` header (Task 8). Errors are JSON `{ "error": string }` with 400/404/409/500.

- [ ] **Step 1: Write failing tests** with `axum::Router` driven in-process: `spawn_then_snapshot_contains_agent`; `events_stream_delivers_seq_ordered_events`; `reconnect_with_last_event_id_gets_only_missed_events`; `reconnect_with_trimmed_log_gets_snapshot_event`; `unknown_agent_is_404`; `terminal_stream_is_read_only` (no write route exists for it); `daemon_refuses_non_loopback_listen`; `ui_schema_route_returns_default_schema`; `openapi_json_matches_fleet_api`.
- [ ] **Step 2: Run** `cargo test -p fleet-daemon --test http`. Expected: FAIL.
- [ ] **Step 3: Implement routes and `main.rs`** (`fleet-daemon --config <path>`, default from `config::default_path`); call `reconcile` once before serving.
- [ ] **Step 4: Run again.** Expected: PASS.

### Task 11: Web UI

**Files:**
- Create: `web/` (Vite + React + TypeScript), `web/src/api/{client,events}.ts`, `web/src/components/{Board,Column,Card,ProjectFilter,SpawnForm,PreviewPane}.tsx`
- Test: `web/src/**/*.test.tsx` (Vitest + Testing Library)

**Interfaces:**
- Consumes: generated types in `web/src/api/generated/`; the routes from Task 10.
- Consumes also: the fleet endpoint comes from configuration (`window.__FLEET_URL__` or `VITE_FLEET_URL`, default same-origin), so the UI never assumes one localhost daemon (`docs/concept.md`); cards show the `blocked_reason` badge via the schema field.
- Produces: `function useUiSchema(): UiSchema` (fetched once from `/api/ui-schema`); `function visibleActions(schema: UiSchema, agent: Agent): ActionDef[]`; `function columnFor(schema: UiSchema, agent: Agent): ColumnDef`; `class FleetClient { snapshot(): Promise<Snapshot>; spawn(project: string, prompt: string): Promise<Agent>; answer(id: string, text: string): Promise<void>; pause(id: string): Promise<void>; resume(id: string): Promise<void>; kill(id: string): Promise<void> }` always sending `actor: "local"`; `function subscribe(onEvent: (e: Event) => void, onSnapshot: (s: Snapshot) => void): () => void` (EventSource with reconnect); `function boardReducer(state: Snapshot, e: Event): Snapshot`.

- [ ] **Step 1: Write failing tests**: `boardReducer_applies_status_changed_and_removed`; `Board_renders_columns_from_schema_only` (a schema with two custom columns renders exactly those two, proving nothing is hard-coded); `columnFor_uses_schema_status_mapping`; `visibleActions_follows_conditions` (Resume only when paused; Answer only in Needs you); `ProjectFilter_hides_other_projects_and_multi_selects`; `Card_shows_project_task_activity_and_time_waiting_for_needs_you`; `Card_shows_blocked_badge_only_in_needs_you` (driven by the schema field, not a hard-coded status); `Card_answer_posts_text_and_keeps_literal_characters` (text `$(x) "q"` sent unmodified); `subscribe_replaces_state_on_snapshot_event`; `PreviewPane_renders_text_stream_read_only` (no input element).
- [ ] **Step 2: Run** `pnpm --dir web test`. Expected: FAIL.
- [ ] **Step 3: Implement the client and components.** Components read titles, fields, actions and tones from the schema; `Tone` selects a CSS token, so a column with `Attention` tone is visually distinct. No component may mention a status or column name.
- [ ] **Step 4: Run** `pnpm --dir web test` and `pnpm --dir web build`. Expected: PASS and `web/dist` exists.

### Task 12: Embed the UI, cross-compile, acceptance run

**Files:**
- Modify: `crates/fleet-daemon/src/http.rs` (serve embedded `web/dist` at `/`); Create: `build.sh` (or `justfile`), `docs/acceptance.md`
- Test: `crates/fleet-daemon/tests/embed.rs`

**Interfaces:**
- Consumes: `web/dist` from Task 11.
- Produces: one self-contained `fleet-daemon` binary; targets `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`.

- [ ] **Step 1: Write failing test** `serves_index_html_and_falls_back_for_client_routes`.
- [ ] **Step 2: Run** `cargo test -p fleet-daemon --test embed`. Expected: FAIL.
- [ ] **Step 3: Embed `web/dist` with rust-embed**; `build.sh` runs the web build, regenerates TS types, then `cargo build --release` for both targets (cross-compiling tool is the builder's choice; document the Nexus mirror needs for crates and npm in `docs/acceptance.md`).
- [ ] **Step 4: Run** `./build.sh`. Expected: both binaries exist and `file` reports the right architecture for each.
- [ ] **Step 5: Run the acceptance checklist in `docs/acceptance.md`** with the real `claude`: spawn agents in two configured repos from the UI; watch cards go Working → Needs you (on a permission prompt) → Working (after you approve in `tmux attach`) → Review; filter by project; answer from a card; pause and resume; kill; restart the daemon while two agents run and confirm both cards survive. Expected: every box ticked, with any card that showed a wrong status recorded as a bug.

---

## Self-review

- **Spec coverage:** board, filter, spawn, per-card answer/pause/kill, read-only preview (Tasks 10–11); tmux per agent and worktrees (6, 7, 9); hook-driven status (3, 8); owner and blocked reason (2, 3, 9, 11); contract with seq/resume, OpenAPI, TS types (2, 4, 10); actor/project on commands (2, 11); loopback, no auth, config file (5); embedded UI and Linux targets (12); Phase 0 spike first (1); login, messaging, bulk approve, orchestrator, egui, multi-user, release pipeline deferred to `TODO.md`.
- **Type consistency:** `Status`, `Agent`, `Event`, `Snapshot`, `HookEvent`, `AgentState`, `BlockedReason` are defined once (Tasks 2–3) and reused by name later.
- **Left to the builder, per spec:** module internals, REST error shapes beyond Task 10, DB schema, worktree location (defaults to `state_dir`), the SQLite and cross-compile tooling.

---

## Amendment A1 (2026-10-23): a project is an idea, repos are its workspaces

**Decided by Antonio:** a project is an idea, one agent session. It starts with exactly one repo and can need more repos while it runs, so repos must be added mid-session. The plan so far treats "project" as a configured repo and gives each agent exactly one worktree. This amendment changes the contract shape and the on-disk layout **in v1**, before Task 12's acceptance run, so the HTTP API and the UI are not built further on the old shape. The flow that adds repos (`request_repo`, approval, write guard, dirty-checkout scan) is **v1.x** and is in `TODO.md`. Rationale and the v1.x design are in `docs/concept.md`.

Run it after Task 11 and before Task 12.

### Vocabulary

- **Repo:** a configured git repository (`config.toml` `repos`). Unchanged.
- **Project:** the idea an agent works on. Its title is `Agent.task` (the first line of the prompt). One agent session = one project.
- **Workspace:** one repo checked out for one project: a worktree on the project's branch.

The type and routes keep the name `Agent` (`/api/agents`). An agent is the session that runs a project, and renaming it touches every file for no behaviour change.

### Task 13: Workspaces in the contract and the per-project layout

**Files:**
- Modify: `crates/fleet-api/src/{model,event,requests,ui,routes}.rs`, `crates/fleet-daemon/src/{git,agents,ingest,http}.rs`, `web/src/api/client.ts`, `web/src/components/{Card,ProjectFilter,SpawnForm}.tsx` (rename `ProjectFilter` to `RepoFilter`)
- Test: `crates/fleet-api/tests/contract.rs`, `crates/fleet-daemon/tests/{git,agents,http}.rs`, `web/src/components/components.test.tsx`

**Interfaces:**
- Produces:
  - `struct Workspace { repo: String, branch: String, worktree: String, base: String, has_changes: bool }` (`base` is the commit the branch started from)
  - `Agent` loses `project`, `branch`, `worktree`; gains `workdir: String` (the project directory, `claude`'s cwd) and `workspaces: Vec<Workspace>` (exactly one in v1: the starting repo, first in the list). `Agent.has_changes` stays and means "any workspace has changes", so `Condition::HasChanges` is unchanged.
  - `SpawnRequest.project` is renamed `repo` (the starting repo).
  - `EventKind::WorkspacesChanged { id: AgentId, workspaces: Vec<Workspace> }`, emitted when a workspace is added or its `has_changes` flips. In v1 only the flip happens.
  - `GET /api/projects` is renamed `GET /api/repos` (the configured repo names).
  - `default_ui_schema()`: the `project` card field is replaced by `repos` (`FieldKind::Badge`, one badge per workspace repo).
  - `git::create(repo: &Path, prefix: &str, slug: &str, dest: &Path) -> Result<Worktree>`: `dest` is the exact worktree path; the caller picks it.
- On disk: `<state_dir>/worktrees/<project-dir>/` is the project directory and `claude`'s cwd. It is a plain directory, not a repo. Each workspace is `<project-dir>/<repo-name>/`. `<project-dir>` is the task slug, deduplicated with `-2`, `-3` across projects. The branch is `<prefix>/<project-dir>` in every repo of the project. A repo added later (v1.x) becomes a sibling directory, already inside the session's cwd, so no `/add-dir` and no restart are needed.
- Trust: the project directory is under the trusted worktrees root, so the spike's inherited-trust finding still applies.
- `kill` removes every workspace's worktree and branch, then the project directory. Spawn failure undoes in reverse order, as before.

- [ ] **Step 0: Verify on the real CLI**, and record the result in `docs/superpowers/findings/hooks-spike.md`: started in a project directory one level above a worktree, (a) does a fresh project directory under the trusted root open with no trust dialog; (b) is the workspace's `CLAUDE.md` loaded at startup, or only when the agent first reads a file there; (c) do hooks fire as before, with `cwd` = the project directory. If (b) is "not at startup", the daemon writes `<project-dir>/CLAUDE.md` containing one `@<repo-name>/CLAUDE.md` import per workspace that has one. This file belongs to the daemon and is not inside any repo. If (a) or (c) fails, stop and bring the finding to Antonio.
- [ ] **Step 1: Write failing tests**:
  - `contract.rs`: `agent_round_trips_with_workspaces`, `workspaces_changed_event_round_trips`, `spawn_request_uses_repo`, `default_ui_schema_has_repos_field_not_project`, `openapi_lists_api_repos`.
  - `git.rs`: `create_puts_worktree_at_dest`.
  - `agents.rs`: `spawn_creates_project_dir_with_one_workspace` (cwd of the tmux session is the project dir; the worktree is `<project-dir>/<repo>`); `same_slug_twice_gets_distinct_project_dirs_and_branches`; `kill_removes_all_workspaces_and_project_dir`; `spawn_failure_leaves_no_project_dir`; `has_changes_is_any_workspace` (a change in the workspace flips both `Workspace.has_changes` and `Agent.has_changes` and emits `WorkspacesChanged`); `reconcile_after_restart_keeps_workspaces`.
  - `http.rs`: `get_repos_lists_configured_repos`; `spawn_with_repo_field`.
  - web: `RepoFilter_shows_agents_touching_any_selected_repo`; `Card_shows_one_badge_per_workspace_repo`; `boardReducer_applies_workspaces_changed`; `SpawnForm_posts_repo`.
- [ ] **Step 2: Run** `cargo test --workspace` and `pnpm --dir web test`. Expected: FAIL on the new tests.
- [ ] **Step 3: Implement.** Agents are stored as JSON in `agents.data`, so no migration is needed. Existing rows from earlier test runs may be dropped: there is no installed base.
- [ ] **Step 4: Run** `cargo test --workspace`, `pnpm --dir web test`, `pnpm --dir web typecheck` and `pnpm --dir web build`. Expected: PASS.
- [ ] **Step 5: Update Task 12's acceptance checklist** (`docs/acceptance.md`): filter by repo, not by project; after spawning, `ls <state_dir>/worktrees/<project-dir>` shows exactly the starting repo.

### Out of this amendment (v1.x, in `TODO.md`)

- The MCP tools `list_repos` and `request_repo(name, why)`, `BlockedReason::RepoAccess`, and the approval that adds a workspace.
- A `PreToolUse` guard that denies file edits outside the project's workspaces and points the agent to `request_repo`.
- A scan of configured repos' main checkouts that flags them when they become dirty while projects run. This catches Bash writes the guard cannot see, and the flag says which agent did it cannot be known for certain.
- Merge order across a project's repos in Review.
