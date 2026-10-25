# Projects View and Project Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace Kill with Stop (agent ends, disk kept) and Archive (deliberate cleanup, record kept forever), show stopped projects in a first Kanban column, and add a Projects view with a passport per project, disk footprint and orphan detection.

**Architecture:** The `agents` table stays the board: a stopped project is still an agent row with `status = stopped` and a `stop_cause`. A new `projects` table holds one record per project that is never deleted (log, per-status time, footprint, last screen, archive report); Archive removes the agent row (the card leaves the board) and marks the record archived. Footprint measurement and the orphan scan run in the background and are cached; the GUI reads them through `GET /api/projects`, polled while the app runs.

**Tech Stack:** Rust workspace (axum 0.8 SSE daemon, sqlx SQLite, tmux, git CLI), egui/eframe 0.37 with egui_kittest tests, ts-rs bindings for the React web UI.

**Spec:** `docs/superpowers/specs/2026-10-24-projects-view-design.md`

## Global Constraints

- Never delete without showing exactly what goes and a confirmation; never delete a branch; never touch anything outside the project's recorded folder and the daemon's own files in `state_dir`; never delete a project record.
- Archive runs **only from the Projects view**, only for a project whose status is `stopped`.
- Stop causes and badge text, verbatim: **Stopped** (amber), **Exited** (blue), **Crashed** (red). Reason lines: `stopped by you`, `agent exited`, `crashed while <last activity>`.
- Board columns, in order: **Stopped, Queued, Working, Needs you, Review**. No Talking column; `Status::Talking` stays in the API.
- Every card on the board has one fixed size.
- Footer copy: `<project> crashed: <last activity>` (warning), `<project> stopped` (success), `<project> exited` (info), `<project> archived, <size> freed` or `<project> archived, kept <path>` (success).
- Tokens and cost show `not tracked yet` (#57). No Resume, rename, partial cleanup or web Projects view.
- Disk sizes are measured off the request path and cached with the time measured; no API call or frame waits on a walk.
- Air-gapped: no new crates from outside the workspace's current dependency set unless already vendored in `Cargo.lock`.
- Repo rules (CLAUDE.local.md): work in a worktree (`git worktree add ../fleet-board-projects -b feat/projects-view`); stage by path; commit subjects Conventional, imperative, under 50 characters; commit and push only when Antonio says so. `cargo fmt --all --check` must stay clean (build.sh checks it). Run tests with `--no-fail-fast`. cargo is at `$HOME/.cargo/bin`.

## Review Focus

1. **A dirty worktree at Archive**: uncommitted files are listed with their count in the plan and lost only after confirm; the branch and its commits survive (test in Task 7).
2. **The daemon restarts while a project is stopped or mid-archive**: a stopped card comes back stopped with its cause (not "unverified"); a half-done archive leaves the record not archived and the plan shows what is still there (tests in Tasks 3 and 7).
3. **A project folder under an old projects root**: still measured, archived from its recorded path, and never reported as an orphan of the new root (test in Task 6).
4. **A symlink inside a worktree pointing outside it** (for example `node_modules` links): the footprint walk does not follow it and Archive does not delete through it (tests in Tasks 5 and 7).
5. **A hook or MCP call arriving for a stopped or archived agent** is refused and changes nothing (test in Task 1, extended in Task 7).

---

## Part A: lifecycle and board (shippable on its own)

### Task 1: Stopped status with a cause

**Files:**
- Modify: `crates/fleet-api/src/status.rs`, `crates/fleet-api/src/model.rs`, `crates/fleet-api/src/event.rs`, `crates/fleet-api/src/lib.rs`
- Modify: `crates/fleet-daemon/src/status.rs:95-140`, `crates/fleet-daemon/src/ingest.rs:50`, `crates/fleet-daemon/src/agents.rs` (reconcile ~1160, `running`, every `Status::Dead`), `crates/fleet-daemon/src/store.rs` (`visible` tuple and `StatusChanged` in `update_agent`)
- Modify: `crates/fleet-client/src/stream.rs` (`apply` for the new event fields)
- Test: `crates/fleet-daemon/tests/status.rs`, `crates/fleet-daemon/tests/agents.rs`, `crates/fleet-api/tests/contract.rs`; update every `Agent { .. }` literal and `Status::Dead` in tests across crates

**Interfaces:**
- Produces:
  - `Status::Stopped` replaces `Status::Dead`, with `#[serde(alias = "dead")]` so stored rows and old events still parse; serialises as `"stopped"`. `Status::ALL` keeps 6 entries.
  - `pub enum StopCause { Stopped, Exited, Crashed }` (snake_case serde, `ToSchema`, `TS`), exported from `fleet_api`.
  - `Agent.stop_cause: Option<StopCause>` and `Agent.disk_bytes: Option<u64>` (`#[ts(type = "number | null")]`), both `#[serde(default)]`.
  - `EventKind::StatusChanged` gains `stop_cause: Option<StopCause>` and `disk_bytes: Option<u64>` (`#[serde(default)]`); `Store::update_agent` emits it when either changes.
  - `fleet_daemon::status::reduce`: `HookEvent::SessionEnd` → `(Status::Stopped, "agent exited")`; the caller sets `stop_cause = Some(Exited)`. Any event after `Stopped` returns `None`.
  - Reconcile on a lost session: `status = Stopped`, `stop_cause = Some(Crashed)`, `activity = format!("crashed while {prev_activity}")`.
  - Startup migration `AgentService::adopt_legacy_dead(&self)`: rows that parsed from `"dead"` with no `stop_cause` get `Exited` when activity is `"session ended"`, else `Crashed` (activity rewritten to `crashed while <activity>` only if it is not already).

- [ ] **Step 1: Write the failing tests**

```rust
// tests/status.rs
#[test]
fn session_end_stops_with_exited() {
    let next = reduce(&working(), &HookEvent::SessionEnd).unwrap();
    assert_eq!(next.status, Status::Stopped);
    assert_eq!(next.activity, "agent exited");
}
#[test]
fn nothing_moves_a_stopped_agent() {
    let stopped = AgentState { status: Status::Stopped, ..working() };
    assert_eq!(reduce(&stopped, &HookEvent::UserPromptSubmit), None);
}
// tests/agents.rs
#[tokio::test]
async fn a_lost_session_is_stopped_as_crashed_with_its_last_activity() {
    // spawn, set activity "running Bash", kill the tmux session behind the daemon's back, reconcile
    assert_eq!(a.status, Status::Stopped);
    assert_eq!(a.stop_cause, Some(StopCause::Crashed));
    assert_eq!(a.activity, "crashed while running Bash");
}
#[tokio::test]
async fn a_stored_dead_agent_is_read_as_stopped_with_its_cause() {
    // insert raw JSON with "status":"dead","activity":"session ended" and another with "session lost"
    // run adopt_legacy_dead
    assert_eq!((ended.status, ended.stop_cause), (Status::Stopped, Some(StopCause::Exited)));
    assert_eq!((lost.status, lost.stop_cause), (Status::Stopped, Some(StopCause::Crashed)));
}
#[tokio::test]
async fn a_hook_for_a_stopped_agent_changes_nothing() { /* ingest SessionStart after Stopped → IngestOutcome::Ignored, row unchanged */ }
// crates/fleet-api/tests/contract.rs
#[test]
fn dead_still_parses_as_stopped() {
    assert_eq!(serde_json::from_str::<Status>("\"dead\"").unwrap(), Status::Stopped);
    assert_eq!(serde_json::to_string(&Status::Stopped).unwrap(), "\"stopped\"");
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p fleet-daemon -p fleet-api --no-fail-fast 2>&1 | grep -E "^test .*FAILED|error\[" | head`
Expected: compile errors (`Status::Stopped`, `StopCause` not found).

- [ ] **Step 3: Implement the types, the reducer and reconcile changes, `adopt_legacy_dead` (called in `main.rs` right after `svc.reconcile()`), and fix every `Status::Dead` and `Agent` literal across the workspace**

`ingest.rs` sets `a.stop_cause = Some(StopCause::Exited)` when `next.status == Status::Stopped`; the repo-request guard compares against `Status::Stopped`.

- [ ] **Step 4: Run the workspace tests**

Run: `cargo test --workspace --no-fail-fast 2>&1 | grep -E "test result|FAILED|panicked" | sort | uniq -c`
Expected: every `test result: ok`.

- [ ] **Step 5: Regenerate the TypeScript bindings and run the web tests**

Run: `cargo test -p fleet-api --test contract ts_bindings_export && pnpm --dir web test`
Expected: `web/src/api/generated/` has `StopCause.ts` and `"stopped"` in `Status.ts`; web tests pass (fix any fixture that uses `"dead"`).

- [ ] **Step 6: Commit** — `feat(daemon): stop agents with a cause`

---

### Task 2: Project records that outlive agents

**Files:**
- Create: `crates/fleet-daemon/migrations/003_projects.sql`, `crates/fleet-api/src/project.rs`
- Modify: `crates/fleet-api/src/lib.rs`, `crates/fleet-daemon/src/store.rs`, `crates/fleet-daemon/src/agents.rs` (spawn, attach_repo, grant_repo, refuse_repo, mark_unverified), `crates/fleet-daemon/src/main.rs`
- Test: `crates/fleet-daemon/tests/store.rs`, `crates/fleet-daemon/tests/agents.rs`

**Interfaces:**
- Consumes: `StopCause`, `Status::Stopped` (Task 1).
- Produces (in `fleet_api`, all `Serialize, Deserialize, ToSchema, TS`):
  - `pub enum ProjectState { Running, Stopped, Archived }`
  - `pub struct LogEntry { pub at: i64, pub text: String }`
  - `pub struct StatusTimes { pub queued: i64, pub working: i64, pub needs_you: i64, pub review: i64 }` (seconds)
  - `pub struct FootprintItem { pub label: String, pub path: String, pub bytes: u64, pub build_bytes: Option<u64> }`
  - `pub struct Footprint { pub measured_at: i64, pub items: Vec<FootprintItem>, pub total: u64 }`
  - `pub struct ArchiveReport { pub removed: Vec<String>, pub freed: u64, pub kept: Option<String> }`
  - `pub struct Project { pub id: AgentId, pub name: String, pub task: String, pub workdir: String, pub repos: Vec<Workspace>, pub state: ProjectState, pub stop_cause: Option<StopCause>, pub created_at: Option<i64>, pub stopped_at: Option<i64>, pub archived_at: Option<i64>, pub agents_used: u32, pub times: StatusTimes, pub log: Vec<LogEntry>, pub footprint: Option<Footprint>, pub last_screen: Option<String>, pub archive: Option<ArchiveReport> }` — `name` is the project folder's basename.
- Produces (in `Store`):
  - `pub async fn projects(&self) -> Result<Vec<Project>>` (all, archived included, ordered by `created_at` then rowid)
  - `pub async fn project(&self, id: &AgentId) -> Result<Option<Project>>`
  - `pub async fn update_project(&self, id: &AgentId, f: impl FnOnce(&mut Project)) -> Result<()>`
  - `pub async fn project_log(&self, id: &AgentId, at: i64, text: &str) -> Result<()>`
  - `insert_agent` also inserts the project (`state Running`, `created_at = status_since`, `agents_used = 1`, log `started with <repos or "no repos">`) in the same transaction.
  - `update_agent`, in the same transaction, when `status` changes: adds `new.status_since - old.status_since` to the old status's bucket in `times` (Talking and Stopped are not counted); on a change to `Stopped` sets `state Stopped`, `stop_cause`, `stopped_at`, and logs `agent exited` / `crashed while …` for those causes (Stop logs its own line in Task 3). Workspaces changes are copied to `repos`.
  - `pub async fn backfill_projects(&self) -> Result<usize>`: one record per agent with no project row (`created_at None`, log `record created on upgrade`); called from `main.rs` after `adopt_legacy_dead`.
- Log lines written by `AgentService`: `repo <name> added`, `repo <name> granted`, `repo request refused: <reason>`, `daemon restarted, status unverified`.

- [ ] **Step 1: Write the failing tests**

```rust
// tests/store.rs
#[tokio::test]
async fn inserting_an_agent_creates_its_project_record() {
    store.insert_agent(&agent("a1", Status::Queued, 100), "t").await.unwrap();
    let p = store.project(&AgentId("a1".into())).await.unwrap().unwrap();
    assert_eq!((p.state, p.created_at, p.agents_used), (ProjectState::Running, Some(100), 1));
    assert_eq!(p.log[0].text, "started with core");
}
#[tokio::test]
async fn status_time_accumulates_per_status() {
    // Queued@100 → Working@130 → NeedsYou@200 → Working@260
    assert_eq!(p.times, StatusTimes { queued: 30, working: 70, needs_you: 60, review: 0 });
}
#[tokio::test]
async fn removing_the_agent_keeps_the_project() {
    store.remove_agent(&id).await.unwrap();
    assert!(store.project(&id).await.unwrap().is_some());
}
#[tokio::test]
async fn backfill_creates_one_record_per_agent_without_one() { /* raw insert into agents only; backfill → 1, then 0 */ }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p fleet-daemon --test store --no-fail-fast`
Expected: compile error (`project` not found).

- [ ] **Step 3: Implement the migration (`CREATE TABLE IF NOT EXISTS projects (id TEXT PRIMARY KEY, data TEXT NOT NULL)`), the types and the `Store` methods; add the four `AgentService` log lines**

- [ ] **Step 4: Run the daemon tests**

Run: `cargo test -p fleet-daemon --no-fail-fast 2>&1 | grep -E "test result|FAILED" | sort | uniq -c`
Expected: all ok.

- [ ] **Step 5: Commit** — `feat(daemon): keep a record for every project`

---

### Task 3: Stop replaces Kill

**Files:**
- Modify: `crates/fleet-daemon/src/agents.rs` (`kill` → `stop`; keep the removal code for Task 7 as private `remove_project_files`), `crates/fleet-daemon/src/http.rs` (routes, `terminal`), `crates/fleet-api/src/routes.rs`, `crates/fleet-api/src/requests.rs` (drop `KillReport`), `crates/fleet-api/src/lib.rs`
- Modify: `crates/fleet-client/src/http.rs` (`kill` → `stop`, `run`), `crates/fleet-daemon/src/main.rs` (`mark_unverified` skips stopped agents)
- Test: `crates/fleet-daemon/tests/agents.rs`, `crates/fleet-daemon/tests/http.rs`, `crates/fleet-client/tests/http.rs`, `crates/fleet-daemon/tests/client_e2e.rs`

**Interfaces:**
- Consumes: `Store::update_project`, `Store::project_log` (Task 2).
- Produces:
  - `pub async fn stop(&self, id: &AgentId) -> Result<(), ServiceError>`: if running, captures the pane (`tmux.capture`) into `Project.last_screen`, kills the session, drops the waiter; sets `status Stopped`, `stop_cause Stopped`, `activity "stopped by you"`, `status_since now`; logs `stopped by you`. On an already-stopped agent: `Ok(())`, nothing changes. Disk untouched.
  - Route `POST /api/agents/{id}/stop` → `204`; `DELETE /api/agents/{id}` is removed (→ 405).
  - `terminal` for a stopped agent: sends `Project.last_screen` (or `"not saved"`) as one `pane` event and ends the stream.
  - `diff` works for a stopped agent (worktrees still on disk).
  - `FleetClient::stop(&self, id: &str) -> Result<(), FleetError>`; `run` no longer special-cases `kill`.

- [ ] **Step 1: Write the failing tests**

```rust
// tests/agents.rs
#[tokio::test]
async fn stop_ends_the_session_and_keeps_everything_on_disk() {
    svc.stop(&id).await.unwrap();
    assert!(!tmux.has_session(&a.tmux_session).await.unwrap());
    assert!(Path::new(&a.workspaces[0].worktree).is_dir());
    assert!(Path::new(&a.workdir).join("CLAUDE.md").is_file());
    let a = find(&svc, &id).await;
    assert_eq!((a.status, a.stop_cause, a.activity.as_str()), (Status::Stopped, Some(StopCause::Stopped), "stopped by you"));
    let p = store.project(&id).await.unwrap().unwrap();
    assert!(p.last_screen.unwrap().contains(MARKER)); // MARKER printed by the test double
    assert_eq!(p.log.last().unwrap().text, "stopped by you");
}
#[tokio::test]
async fn stopping_twice_is_a_no_op() { /* second stop Ok, no new event */ }
#[tokio::test]
async fn a_stopped_agent_stays_stopped_across_a_daemon_restart() { /* new AgentService over the same store; reconcile + mark_unverified; status/cause unchanged, activity still "stopped by you" */ }
// tests/http.rs
#[tokio::test]
async fn delete_agent_is_gone_and_post_stop_stops() { /* DELETE → 405; POST /stop → 204 */ }
#[tokio::test]
async fn the_terminal_of_a_stopped_agent_sends_its_saved_screen_once() { /* one pane event == last_screen, then end */ }
#[tokio::test]
async fn the_diff_of_a_stopped_agent_still_works() {}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p fleet-daemon -p fleet-client --no-fail-fast 2>&1 | grep -E "FAILED|error\[" | head`
Expected: failures / missing `stop`.

- [ ] **Step 3: Implement `stop`, the route, `terminal` and `diff` changes and the client; replace every Kill test with its Stop equivalent (the folder-removal tests move to Task 7)**

- [ ] **Step 4: Run the workspace tests**

Run: `cargo test --workspace --no-fail-fast 2>&1 | grep -E "test result|FAILED" | sort | uniq -c`
Expected: all ok.

- [ ] **Step 5: Commit** — `feat(daemon): replace Kill with Stop`

---

### Task 4: The board: Stopped column, cause badges, fixed cards, footer

**Files:**
- Modify: `crates/fleet-api/src/ui.rs` (`Tone`, `FieldKind`, `ColumnDef`, `FieldDef`, `default_ui_schema`)
- Modify: `crates/fleet-gui/src/theme.rs` (colours for the new tones), `crates/fleet-gui/src/widgets/badge.rs`, `crates/fleet-gui/src/components/card.rs`, `crates/fleet-gui/src/components/column.rs`, `crates/fleet-gui/src/components/preview.rs`, `crates/fleet-gui/src/app.rs` (`handle_stream` notices)
- Modify: `docs/acceptance-gui.md`, `docs/acceptance.md` (Kill → Stop, Stopped column)
- Test: `crates/fleet-api/tests/contract.rs`, `crates/fleet-gui/tests/board.rs`, `crates/fleet-gui/tests/panels.rs`, `crates/fleet-gui/tests/app.rs`, web tests

**Interfaces:**
- Consumes: `Status::Stopped`, `StopCause`, `Agent.stop_cause`, `Agent.disk_bytes` (Task 1); `POST /api/agents/{id}/stop` (Task 3).
- Produces:
  - `Tone::Caution` (amber) and `Tone::Danger` (red) added; `FieldKind::Bytes`, rendered by `pub fn format_bytes(bytes: u64) -> String` in `crates/fleet-gui/src/widgets/mod.rs` (SI units, one decimal: `7.6 GB`, `12.0 KB`; `0 B` for zero).
  - `ColumnDef.dim: bool` (`#[serde(default)]`): the GUI fills a dimmed column with a darker background.
  - `FieldDef.value_tones: BTreeMap<String, Tone>` (`#[serde(default)]`): a badge's tone by raw value.
  - `default_ui_schema()`: columns `stopped` ("Stopped", `[Stopped]`, hint "Nothing stopped", `Tone::Muted`, `dim: true`), `queued`, `working`, `needs_you`, `review` (`[Review]` only); no `talking`. Card field `stop_cause` badge, labels `stopped→Stopped`, `exited→Exited`, `crashed→Crashed`, tones `Caution`, `Active`, `Danger`, shown when `StatusIn [Stopped]`; field `disk_bytes` kind `Bytes` shown when `StatusIn [Stopped]`. The old `status`/"Ended" badge is removed. Action `stop` ("Stop", `POST`, `/api/agents/{id}/stop`, `destructive: true`, when `Not(StatusIn [Stopped])`); `kill` removed; `resume` condition drops `Talking`.
  - `pub const CARD_HEIGHT: f32` in `card.rs`: every card is exactly this tall; slots: duration row, title (≤ 2 lines, then `…`), one badge row (overflow → `+N` badge with the rest in its tooltip), one activity line (`…`, full text in tooltip), one action row. Truncated text shows whole on hover.
  - Preview for a stopped agent: no reply box; Terminal shows the saved screen (from the stream); Diff as usual.
  - Footer notices from `handle_stream` on a `StatusChanged` into `Stopped`: `tracing::warn!(notify = "warning", "{task} crashed: {last activity}")` for `Crashed` (clicking it selects the card: the message carries the agent id), `tracing::info!(notify = "info", "{task} exited")` for `Exited`; the Stop action's success posts `"{task} stopped"` with `notify = "success"`.

- [ ] **Step 1: Write the failing tests**

```rust
// contract.rs
#[test]
fn the_board_starts_with_stopped_and_has_no_talking() {
    let ids: Vec<_> = default_ui_schema().columns.iter().map(|c| c.id.clone()).collect();
    assert_eq!(ids, ["stopped", "queued", "working", "needs_you", "review"]);
    assert!(default_ui_schema().columns[0].dim);
}
#[test]
fn stop_replaces_kill_and_hides_on_stopped_cards() { /* "stop" action present, "kill" absent, eval(when, stopped agent) == false */ }
// board.rs
#[test]
fn every_card_has_the_same_height() {
    // one card with a 300-char task, three badges and a long activity; one with a 3-word task and nothing else
    assert_eq!(rect_of("long").height(), rect_of("short").height());
    assert_eq!(rect_of("short").height(), CARD_HEIGHT);
}
#[test]
fn a_stopped_card_shows_its_cause_badge_and_size() { /* "Crashed" label, "7.6 GB" label, no "Stop" button */ }
// panels.rs
#[test]
fn a_stopped_agents_panel_has_no_reply_box() {}
// app.rs
#[test]
fn a_crash_posts_a_warning_naming_the_project_and_last_activity() { /* inject StatusChanged → footer text "fix login crashed: crashed while running Bash" */ }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p fleet-api -p fleet-gui --no-fail-fast 2>&1 | grep -E "FAILED|error\[" | head`
Expected: failures.

- [ ] **Step 3: Implement the schema, tones, `CARD_HEIGHT` layout, dimmed column, preview and notices; update the acceptance checklists (Kill → Stop; a Stopped column item; fixed card size item)**

- [ ] **Step 4: Run tests, regenerate bindings, web tests, fmt, clippy**

Run: `cargo test --workspace --no-fail-fast 2>&1 | grep -E "test result|FAILED" | sort | uniq -c && cargo test -p fleet-api --test contract ts_bindings_export && pnpm --dir web test && cargo fmt --all --check && cargo clippy --workspace --all-targets -q`
Expected: all ok, no clippy output.

- [ ] **Step 5: Look at it**: run the app on Xvfb as in `docs/superpowers/findings/app-shell.md` (scratch HOME, test-double claude), spawn two agents, stop one, kill the other's tmux session by hand; screenshot the board. Expected: Stopped column first with a Stopped and a Crashed card of equal size, footer warning for the crash.

- [ ] **Step 6: Commit** — `feat(gui): add the Stopped column`

---

## Part B: Projects view

### Task 5: Footprint measurement

**Files:**
- Create: `crates/fleet-daemon/src/footprint.rs`
- Modify: `crates/fleet-daemon/src/lib.rs`, `crates/fleet-daemon/src/agents.rs` (measurer task), `crates/fleet-daemon/src/main.rs`
- Test: `crates/fleet-daemon/tests/footprint.rs`

**Interfaces:**
- Consumes: `Footprint`, `FootprintItem`, `Store::update_project`, `Store::update_agent` (Tasks 1-2).
- Produces:
  - `pub const BUILD_DIRS: [&str; 2] = ["target", "node_modules"];` (looked for at each worktree's root)
  - `pub fn measure(workdir: &Path, worktrees: &[(String, PathBuf)], daemon_files: &Path, now: i64) -> Footprint` (blocking; items: one per worktree labelled `<repo> worktree` with `build_bytes`, one `daemon files` for the agent's settings folder plus `CLAUDE.md` and `PROMPT.md`, one `other files` for anything else in the project folder when non-zero). Walks with `symlink_metadata`, never follows symlinks, counts allocated size as `len()`, skips unreadable entries.
  - `AgentService::request_measure(&self, id: &AgentId)` and `request_measure_all(&self)`: queue onto one background task (one walk at a time, `spawn_blocking`); on completion write `Project.footprint` and `Agent.disk_bytes = Some(total)`. Called at startup (all non-archived), after `stop`, and by `POST /api/projects/refresh` (Task 8).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn measure_splits_build_output_and_daemon_files() {
    // workdir/core/{src/a.rs 10 B, target/x 1000 B}, workdir/CLAUDE.md 5 B, agent_dir/settings.json 7 B
    let f = measure(&wd, &[("core".into(), wd.join("core"))], &agent_dir, 42);
    assert_eq!(f.items[0], FootprintItem { label: "core worktree".into(), path: s(wd.join("core")), bytes: 1010, build_bytes: Some(1000) });
    assert_eq!(f.items[1].label, "daemon files"); assert_eq!(f.items[1].bytes, 12);
    assert_eq!((f.total, f.measured_at), (1022, 42));
}
#[test]
fn measure_does_not_follow_symlinks_out_of_the_worktree() { /* core/link -> big dir outside; bytes excludes it */ }
#[tokio::test]
async fn stop_triggers_a_measurement_that_lands_on_the_card() { /* after stop, eventually agent.disk_bytes is Some and project.footprint is Some */ }
```

- [ ] **Step 2: Run them to verify they fail** — `cargo test -p fleet-daemon --test footprint --no-fail-fast` → compile error.

- [ ] **Step 3: Implement `measure` and the measurer**

- [ ] **Step 4: Run the daemon tests** — `cargo test -p fleet-daemon --no-fail-fast` → all ok.

- [ ] **Step 5: Commit** — `feat(daemon): measure each project's disk use`

---

### Task 6: Orphan scan and End session

**Files:**
- Create: `crates/fleet-daemon/src/orphans.rs`
- Modify: `crates/fleet-api/src/project.rs`, `crates/fleet-daemon/src/agents.rs`, `crates/fleet-daemon/src/main.rs` (scan after `sweep_tmp_clones` and the backfill)
- Test: `crates/fleet-daemon/tests/orphans.rs`

**Interfaces:**
- Consumes: `Store::projects` (Task 2), `fleet_fs::main_root`, `Tmux::list_sessions`, `SESSION_PREFIX`.
- Produces:
  - `pub enum OrphanKind { ProjectFolder, AgentFolder, Worktree, Session }` and `pub struct Orphan { pub id: String, pub kind: OrphanKind, pub path: String, pub worktrees: Vec<String>, pub bytes: Option<u64> }` in `fleet_api`. `id` is `"<kind>:<path or session name>"`. `bytes` is filled by the measurer (Task 5) for folders.
  - `pub fn scan(projects_root: &Path, agents_dir: &Path, bases: &[PathBuf], known_workdirs: &HashSet<PathBuf>, known_agent_ids: &HashSet<String>) -> Vec<Orphan>` (blocking; folders directly under `projects_root` not in `known_workdirs`, with the registered worktrees inside them; agent folders under `agents_dir` whose name is not a known id; worktrees registered in a base under `projects_root` outside any orphan or known folder, including ones whose path is gone). Known = every project record, archived included, so a folder kept by an archive (`kept …`) is not re-reported. [plan call]
  - Sessions: `tmux.list_sessions(SESSION_PREFIX)` minus every agent's `tmux_session` → `OrphanKind::Session` with `path` = session name.
  - `AgentService::orphans(&self) -> Vec<Orphan>` (cached result of the last scan) and `rescan_orphans(&self)`.
  - `AgentService::end_session(&self, session: &str) -> Result<(), ServiceError>`: only a session that is a current orphan; kills it; rescans.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_folder_with_no_record_is_an_orphan_with_its_worktrees() {}
#[test]
fn a_known_project_under_an_old_root_is_not_an_orphan_of_the_new_root() {
    // record workdir /old/root/p1; scan /new/root that contains nothing; also scan with /old/root as root → still not an orphan
}
#[test]
fn an_archived_projects_kept_folder_is_not_reported_again() {}
#[test]
fn a_registered_worktree_whose_folder_is_gone_is_an_orphan() {}
#[tokio::test]
async fn end_session_refuses_a_session_that_belongs_to_an_agent() {}
```

- [ ] **Step 2: Run them to verify they fail** — compile error.
- [ ] **Step 3: Implement `scan`, the cache, the startup scan and `end_session`**
- [ ] **Step 4: Run the daemon tests** — all ok.
- [ ] **Step 5: Commit** — `feat(daemon): find orphans at startup`

---

### Task 7: Archive (plan, confirm, final record)

**Files:**
- Create: `crates/fleet-daemon/src/archive.rs`
- Modify: `crates/fleet-daemon/src/agents.rs` (move Kill's removal code here: `remove_project_worktree`, `remove_if_empty`), `crates/fleet-daemon/src/git.rs`, `crates/fleet-api/src/project.rs`
- Test: `crates/fleet-daemon/tests/archive.rs`

**Interfaces:**
- Consumes: `Footprint` measurement (Task 5), `Orphan` (Task 6), `Store::remove_agent`, `Store::update_project`.
- Produces:
  - `pub struct PlanItem { pub path: String, pub bytes: u64, pub uncommitted_files: u32 }` and `pub struct ArchivePlan { pub items: Vec<PlanItem>, pub total: u64 }` in `fleet_api`.
  - `pub async fn uncommitted_files(wt: &Path) -> Result<u32, GitError>` in `git.rs` (`git status --porcelain` line count).
  - `AgentService::archive_plan(&self, id: &AgentId) -> Result<ArchivePlan, ServiceError>`: only for `status Stopped` (else `ServiceError::NotStopped`); measures fresh; items: each non-owned worktree, the daemon files, the project folder's other contents.
  - `AgentService::archive(&self, id: &AgentId) -> Result<ArchiveReport, ServiceError>`: only for `Stopped`; removes as Kill did (git removal, by-hand fallback only inside the folder, never through a symlink, branches untouched, folder only if empty else `kept <path>: it contains …`); then `store.remove_agent` (card leaves the board) and `update_project`: `state Archived`, `archived_at`, `archive = report`, `footprint` = the plan's measurement; log `archived, <size> freed` or `archived, kept <path>`. If removal fails midway, the agent row and record stay as they were except a log line `archive failed: <error>`; the next plan shows what is left.
  - `AgentService::orphan_plan(&self, orphan_id: &str) -> Result<ArchivePlan, ServiceError>` and `archive_orphan(&self, orphan_id: &str) -> Result<ArchiveReport, ServiceError>` (not for `Session`): same removal; creates an archived `Project` (`id` = new uuid, `name` = folder basename, log `found as orphan`, then the archive line); rescans.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn archive_refuses_a_running_project() { /* NotStopped, nothing removed */ }
#[tokio::test]
async fn the_plan_counts_uncommitted_files() { /* write 2 untracked files in the worktree; plan.items[0].uncommitted_files == 2 */ }
#[tokio::test]
async fn archive_removes_the_worktree_keeps_the_branch_and_the_record() {
    let r = svc.archive(&id).await.unwrap();
    assert!(!Path::new(&wt).exists());
    assert!(git::branch_exists(&base, "fleet/fix-login").await.unwrap());
    assert!(find_opt(&svc, &id).await.is_none());
    let p = store.project(&id).await.unwrap().unwrap();
    assert_eq!(p.state, ProjectState::Archived);
    assert_eq!(p.archive.unwrap(), r);
    assert!(p.log.last().unwrap().text.starts_with("archived, "));
}
#[tokio::test]
async fn archive_keeps_a_folder_holding_the_users_files() { /* notes.txt in workdir → kept == Some("kept …: it contains notes.txt") */ }
#[tokio::test]
async fn archive_never_deletes_through_a_symlink() { /* worktree link -> outside dir; outside dir intact after archive */ }
#[tokio::test]
async fn archiving_an_orphan_folder_creates_an_archived_record() {}
#[tokio::test]
async fn a_hook_for_an_archived_agent_is_refused() { /* ingest → IngestError unknown agent */ }
```

- [ ] **Step 2: Run them to verify they fail** — compile error.
- [ ] **Step 3: Implement**
- [ ] **Step 4: Run the daemon tests** — all ok.
- [ ] **Step 5: Commit** — `feat(daemon): archive stopped projects`

---

### Task 8: Projects API and client

**Files:**
- Modify: `crates/fleet-api/src/routes.rs`, `crates/fleet-api/src/lib.rs` (OpenAPI paths and schemas), `crates/fleet-daemon/src/http.rs`, `crates/fleet-client/src/http.rs`
- Test: `crates/fleet-daemon/tests/http.rs`, `crates/fleet-client/tests/http.rs`, `crates/fleet-api/tests/contract.rs`

**Interfaces:**
- Consumes: Tasks 2, 5, 6, 7.
- Produces:
  - `pub struct ProjectList { pub projects: Vec<Project>, pub orphans: Vec<Orphan>, pub clones_bytes: Option<u64> }` (`clones_bytes` from the measurer over `<state_dir>/repos`).
  - Routes: `GET /api/projects` → `ProjectList`; `POST /api/projects/refresh` → `202` (rescan + measure all); `GET /api/projects/{id}/archive-plan` → `ArchivePlan`; `POST /api/projects/{id}/archive` → `ArchiveReport`; `POST /api/orphans/plan` `{ "id": … }` → `ArchivePlan`; `POST /api/orphans/archive` `{ "id": … }` → `ArchiveReport`; `POST /api/orphans/end-session` `{ "id": … }` → `204`. Not-stopped → `409` with the reason text.
  - `FleetClient`: `projects()`, `refresh_projects()`, `archive_plan(id)`, `archive(id)`, `orphan_plan(id)`, `archive_orphan(id)`, `end_session(id)`; archive calls use `LONG_TIMEOUT`.

- [ ] **Step 1: Write the failing tests** — `projects_lists_archived_and_orphans`, `archive_on_a_running_project_is_409`, `the_openapi_document_lists_the_project_routes`, client round-trips against the test daemon.
- [ ] **Step 2: Run them to verify they fail.**
- [ ] **Step 3: Implement; regenerate bindings.**
- [ ] **Step 4: Run** `cargo test --workspace --no-fail-fast` and `pnpm --dir web test` → all ok.
- [ ] **Step 5: Commit** — `feat(daemon): serve the projects list`

---

### Task 9: The Projects view

**Files:**
- Create: `crates/fleet-gui/src/views/projects.rs`, `crates/fleet-gui/src/components/passport.rs`, `crates/fleet-gui/src/components/archive_dialog.rs`
- Modify: `crates/fleet-gui/src/views/mod.rs` (`View::Projects`, label "Projects", between Kanban and Settings), `crates/fleet-gui/src/drawer.rs`, `crates/fleet-gui/src/app.rs` (state, polling, results), `crates/fleet-gui/src/components/mod.rs`, `crates/fleet-gui/src/components/preview.rs` (reuse the diff view)
- Modify: `docs/acceptance-gui.md` (a Projects section)
- Test: `crates/fleet-gui/tests/projects.rs`

**Interfaces:**
- Consumes: `ProjectList`, client calls (Task 8); `FieldKind::Bytes` formatting helper from Task 4 (`pub fn format_bytes(u64) -> String` in `widgets`).
- Produces:
  - `AppResult::Projects(Result<ProjectList, String>)`, `AppResult::ArchivePlan(String, Result<ArchivePlan, String>)`, `AppResult::Archived(String, Result<ArchiveReport, String>)`.
  - Polling: `GET /api/projects` every `PROJECTS_EVERY = 10 s` and after Stop, Archive, End session, Refresh.
  - `pub enum ProjectFilter { All, Running, Stopped, Crashed, Orphans }`; `FleetApp::open_projects(&mut self, filter: ProjectFilter, select: Option<String>)`.
  - Layout (spec "The Projects view"): header "Projects" with "Show archived" toggle (off); totals row (projects by state, crashed counted on its own; disk held and reclaimable; clones cache; agents started; tokens · cost `—` / `not tracked yet`); list 250 px wide (row: name, cause/status badge, size; order running, stopped with crashed first, orphans ⚠, archived dimmed; newest first within); passport (name, badge, started, folder, Open on board unless archived; goal; repos with branch and start commit; Footprint table with `of which target/` lines, `measured <ago>` and Refresh; Activity: agents used, time running and per status, tokens/cost `not tracked yet`; Log; Diff and Last screen for stopped; Actions: Stop when running, `Archive…` when stopped, disabled with tooltip `stop it first` otherwise).
  - Archive dialog: lists the plan items with sizes and, for a dirty worktree, `N uncommitted files will be lost`; buttons Cancel (focused) and `Archive` (destructive); on success posts `"{name} archived, {size} freed"` or `"{name} archived, kept {path}"` (`notify = "success"`).
  - Orphan rows: Archive… with the same dialog; Session orphans offer `End session…` with the confirmation text `An agent may be working in this session. End it?`.

- [ ] **Step 1: Write the failing tests** (egui_kittest, injected results like `tests/panels.rs`)

```rust
#[test] fn the_drawer_lists_projects_between_kanban_and_settings() {}
#[test] fn the_list_is_narrower_than_the_passport() { assert!(list_rect.width() < passport_rect.width()); }
#[test] fn totals_count_crashed_on_their_own() { /* 1 running, 2 stopped of which 1 crashed → "1 running", "2 stopped", "1 crashed" */ }
#[test] fn archive_is_disabled_until_the_project_is_stopped() {}
#[test] fn the_archive_dialog_warns_about_uncommitted_files_and_focuses_cancel() {}
#[test] fn archived_projects_are_hidden_until_show_archived() {}
#[test] fn tokens_and_cost_say_not_tracked_yet() {}
```

- [ ] **Step 2: Run them to verify they fail.**
- [ ] **Step 3: Implement the view, passport, dialog and polling; add the acceptance section.**
- [ ] **Step 4: Run** `cargo test -p fleet-gui --no-fail-fast`, fmt, clippy → all ok.
- [ ] **Step 5: Commit** — `feat(gui): add the Projects view`

---

### Task 10: Kanban counts and Details

**Files:**
- Modify: `crates/fleet-gui/src/views/kanban.rs` (`header`), `crates/fleet-gui/src/components/card.rs`, `crates/fleet-gui/src/app.rs`
- Test: `crates/fleet-gui/tests/board.rs`, `crates/fleet-gui/tests/app.rs`

**Interfaces:**
- Consumes: `ProjectList` polling, `FleetApp::open_projects` (Task 9).
- Produces:
  - Kanban header, right of the Repo filter: `Projects:` then `● N running`, `● N stopped`, `N crashed` badge (only when > 0, `Tone::Danger`), `⚠ N orphan` (only when > 0), `· <total disk>`; each count calls `open_projects` with its filter.
  - Stopped cards get a GUI-side `Details` button (not from the schema) → `Command::Details(agent_id)` → `open_projects(ProjectFilter::All, Some(id))`.
  - The crash footer message's click selects the card (`Command::Open(id)`).

- [ ] **Step 1: Write the failing tests** — `header_counts_show_crashed_and_orphans_only_when_present`, `clicking_stopped_opens_projects_filtered_to_stopped`, `details_on_a_stopped_card_selects_it_in_projects`.
- [ ] **Step 2: Run them to verify they fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Full verification**: `cargo test --workspace --no-fail-fast`, `pnpm --dir web test`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -q`; then the Xvfb run from Task 4 extended: stop, archive one project (folder gone, branch kept, record archived), create an orphan folder by hand and restart the daemon (⚠ row), screenshots of Kanban and Projects into `docs/superpowers/findings/projects-view/`.
- [ ] **Step 5: Commit** — `feat(gui): show project counts on the board`

After Task 10: close #104 and mention #103 (archive part) in the merge commit body; file follow-ups for anything deferred.
