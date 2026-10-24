# Projects First Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** fleet-board starts with no config. A project is a goal with its own folder under a projects root. Repos (path, git URL or remembered name) are attached per project: at start, by hand, or on the agent's MCP request, which Antonio grants.

**Architecture:**
- The daemon gets an optional config with `projects_root`.
- A SQLite `repos` table is the remembered list.
- A `repos.rs` module resolves sources, keeps one base clone per repo and makes per-project worktrees from the remote default branch.
- `AgentService` spawns into `<projects_root>/<slug>` with zero or more repos, attaches repos later, detects agent-initialised repos, and keeps owned repos on Kill.
- A minimal MCP-over-HTTP endpoint lets the agent `request_repo`, held open until Antonio grants or refuses from the GUI.

**Tech Stack:**
- Rust 2021, axum, sqlx/SQLite, tokio;
- git and tmux CLIs;
- eframe/egui 0.37;
- `egui_kittest`;
- Claude Code CLI (`--settings`, `--mcp-config`).

**Spec:** `docs/superpowers/specs/2026-10-24-projects-first-design.md`. Read it before any task.

## Global Constraints

- **No new crates.** MCP is hand-written JSON-RPC with `serde_json` and axum.
- **The web UI (`web/`) is frozen.**
  - Do not edit `web/src` by hand.
  - Generated files that `cargo test -p fleet-api` rewrites (`web/src/api/generated/*`) may change. Leave those changes in.
- **The daemon stays loopback-only.**
  - `/internal/mcp/{id}` uses the same per-agent `X-Fleet-Token` check as `/internal/hooks/{id}`, through `constant_time_eq`.
  - `<agent_dir>/mcp.json` is written with mode `0600`.
- **Defaults:**
  - `projects_root`: `~/fleet-projects`.
  - URL clone store: `<state_dir>/repos/<name>`.
  - Project folder: `<projects_root>/<slug>`, then `<slug>-2`, …
  - Branch: `<branch_prefix>/<project-folder-name>`.
  - Agent-initialised workspace `base`: git's empty tree `4b825dc642cb6eb9a060e54bf8d69288fbee4904`.
- **Source resolution order:** remembered name → path (starts with `/` or `~`, or is an existing dir) → URL (contains `://` or matches `^[^/@\s]+@[^:\s]+:.+`). Anything else is an error naming the accepted forms.
- **Kill never deletes an agent-initialised repo or any file it didn't create.**
- **No git commit steps.** Antonio commits by hand.
- The existing gates stay green:
  - `cargo test --workspace`;
  - `cargo clippy --workspace --all-targets -- -D warnings -A clippy::large_enum_variant`;
  - the colour-literal grep over `crates/fleet-gui/src` (outside `theme.rs`).
- GUI work follows `docs/ui.md`: tokens and primitives only, motion only through `theme::animate`.

## Review Focus

1. **A URL that fails to clone** (bad host, no credentials, or a name that clashes) must leave no half-made clone dir and no remembered row, and must report git's own message. Test in Task 5.
2. **Two projects attaching the same URL at the same moment** must make exactly one base clone. The second waits and reuses it. Test in Task 5.
3. **A path that is inside the projects root**, e.g. granting another project's worktree, must be refused with a clear error, so worktrees are never nested in worktrees. Test in Task 5.
4. **The daemon restarting while a `request_repo` call is held.** The held call dies with the connection. The card must not stay Needs you / `repo_access` forever: on startup, a stored `repo_request` is cleared and the activity says the request was interrupted. Test in Task 7.
5. **Kill on a project that has an agent-initialised repo plus one granted worktree.** The worktree is removed, the owned repo and the folder stay, and the reply says what was kept. Test in Task 6.

---

### Task 1: Spike — Claude Code MCP over HTTP and held tool calls

**Files:**
- Create: `docs/superpowers/findings/mcp-spike.md`.
- Create: a throwaway script under the scratchpad (not committed).

- [ ] **Step 1: Write a 40-line throwaway MCP server.**
  - Use Python's stdlib `http.server`.
  - It answers `initialize`, `tools/list` (one tool, `wait(seconds)`) and `tools/call`.
  - `tools/call` sleeps `seconds`, then returns text.
  - It requires header `X-Fleet-Token: t1`.
- [ ] **Step 2: Run the real CLI against it in a temp dir.**
  - The config is a temp `mcp.json`: `{"mcpServers":{"fleet":{"type":"http","url":"http://127.0.0.1:<port>/mcp","headers":{"X-Fleet-Token":"t1"}}}}`.
  - Command: `claude --mcp-config <mcp.json> --permission-mode default -- "call the fleet wait tool with seconds=…"`.
  - Answer the permission prompt.
  - Try `seconds` = 30, 120, 600 and 1800.
- [ ] **Step 3: Record** in `mcp-spike.md`, with the Claude Code version:
  - whether the http transport with a header works;
  - the exact request sequence the server saw;
  - the tool's name as Claude reports it in `PreToolUse` (`mcp__fleet__wait`);
  - which hooks fire around the call;
  - how long a call can be held before Claude gives up, and what the agent then sees;
  - whether `MCP_TOOL_TIMEOUT` changes it.
- [ ] **Step 4: State the outcome on the first line of `mcp-spike.md`:**
  - **HOLD** if a 30-minute hold works, optionally with `MCP_TOOL_TIMEOUT` set in the tmux env;
  - **ASYNC** if not, in which case Task 7 uses the spec's fallback.

---

### Task 2: Optional config with `projects_root`

**Files:**
- Modify: `crates/fleet-daemon/src/config.rs`, `crates/fleet-client/src/config.rs`, `packaging/config.example.toml`.
- Test: `crates/fleet-daemon/tests/config.rs`, `crates/fleet-client/tests/config.rs`.

**Interfaces:**
- Produces (daemon `config.rs`):
  - `Config { listen, state_dir, branch_prefix, claude_command, projects_root: PathBuf, seed_repos: Vec<Repo> }`. `repos` is renamed `seed_repos`; its entries are validated as today (name rule, is a dir) but used only for seeding.
  - `load_with_home` returns defaults when the file does not exist (`ErrorKind::NotFound`).
- Produces (client `config.rs`):
  - `ClientConfig` gains `projects_root: PathBuf`.
  - `load` returns defaults for a missing file.
  - `pub fn write_initial(path: &Path, projects_root: &Path, home: &Path) -> std::io::Result<()>` creates the parent dir and writes `projects_root = "<root, with home shown as ~>"` followed by the commented template (`include_str!("../../../packaging/config.example.toml")`).
  - `pub fn exists(path: &Path) -> bool`.

- [ ] **Step 1: Failing tests:**
  - **Daemon:**
    - **`missing_file_is_all_defaults`:** `projects_root == home/"fleet-projects"`, `seed_repos` empty, `listen == 127.0.0.1:7878`.
    - **`projects_root_expands_tilde`.**
    - **`v0_1_repos_load_as_seed_repos`.**
  - **Client:**
    - **`missing_file_is_defaults`.**
    - **`write_initial_then_load_round_trips_the_root`.**
    - **`write_initial_keeps_the_template_comments`:** the file contains `# claude_command`.
- [ ] **Step 2: Run them.** Run `cargo test -p fleet-daemon --test config` and `cargo test -p fleet-client --test config`. Expected: FAIL.
- [ ] **Step 3: Implement.** Add `projects_root` to both `Raw` structs; `deny_unknown_fields` stays on the daemon. Update `config.example.toml` with a commented `# projects_root = "~/fleet-projects"` block and a note that `[[repos]]` is only read once to seed the remembered list.
- [ ] **Step 4: Run** `cargo test -p fleet-daemon -p fleet-client`. Fix compile errors where `cfg.repos` was used by renaming to `seed_repos`; spawn's use is rewritten in Task 6, so make it compile minimally. Expected: PASS.

---

### Task 3: Contract changes in `fleet-api`

**Files:**
- Modify: `crates/fleet-api/src/{model,status,requests,routes,ui,lib}.rs`.
- Test: the existing `fleet-api` tests, plus new ones in `crates/fleet-api/tests/` (follow the file the crate already uses).

**Interfaces:**
- Produces:

```rust
pub struct Workspace { pub repo: String, pub branch: String, pub worktree: String, pub base: String, pub has_changes: bool,
    #[serde(default)] pub owned: bool }
pub struct RepoRequest { pub repo: String, pub why: String }
pub struct Agent { /* existing fields */ #[serde(default)] pub repo_request: Option<RepoRequest> }
pub enum BlockedReason { Permission, Question, Error, Startup, RepoAccess }   // "repo_access"
pub struct SpawnRequest { pub actor: Actor, #[serde(default)] pub repos: Vec<String>, pub prompt: String, pub task_slug: Option<String> }
pub struct RepoInfo { pub name: String, pub source: RepoSource, pub location: String }
pub enum RepoSource { Path, Url }                                            // snake_case
pub struct RepoList { pub repos: Vec<RepoInfo> }
pub struct AddRepoRequest { pub actor: Actor, pub repo: String }
pub struct GrantRepoRequest { pub actor: Actor, pub repo: String }
pub struct RefuseRepoRequest { pub actor: Actor, #[serde(default)] pub reason: String }
pub struct KillReport { pub kept: Option<String> }   // body of DELETE /api/agents/{id}: Some(message) when the folder was kept
pub enum Condition { /* existing */ BlockedReasonIn { reasons: Vec<BlockedReason> },
                     Not { #[schema(no_recursion)] condition: Box<Condition> } }
pub struct FieldDef { /* existing */ #[serde(default)] pub value_labels: std::collections::BTreeMap<String, String> }
```

- Routes (utoipa descriptions in `routes.rs`):
  - `POST /api/agents/{id}/repos` (`AddRepoRequest` → `Workspace`);
  - `POST /api/agents/{id}/repo-request/grant` (`GrantRepoRequest` → `Workspace`);
  - `POST /api/agents/{id}/repo-request/refuse` (`RefuseRepoRequest` → 204);
  - `DELETE /api/agents/{id}` returns 200 with `KillReport`.
- Default schema:
  - The answer action's `when` becomes `All[StatusIn[NeedsYou], Not(BlockedReasonIn[RepoAccess])]`.
  - The `blocked_reason` field gets `value_labels`: `permission→"Permission needed"`, `question→"Question"`, `error→"Error"`, `startup→"Startup prompt"`, `repo_access→"Repo access"`.

- [ ] **Step 1: Failing tests:**
  - **`v0_1_agent_json_without_new_fields_still_parses`:** a literal JSON without `owned` or `repo_request`.
  - **`answer_is_hidden_for_a_repo_access_request`:** `eval` on an agent in NeedsYou + `RepoAccess`.
  - **`answer_shows_for_a_permission_request`.**
  - **`blocked_reason_labels_cover_every_reason`:** every `BlockedReason` variant's serde name has a label.
  - **`spawn_request_without_repos_is_valid`.**
- [ ] **Step 2: Run** `cargo test -p fleet-api`. Expected: FAIL.
- [ ] **Step 3: Implement** the types, `eval` for the two new conditions, the schema changes and the route descriptions. Keep ts-rs and utoipa derives on every new type.
- [ ] **Step 4: Fix downstream compile errors only:**
  - `fleet-daemon` builds `Agent` and `Workspace` literals (add `repo_request: None`, `owned: false`), and `SpawnRequest.repo` users go to `repos` (Task 6 rewrites spawn).
  - In `fleet-client`, `spawn` sends `"repos": [repo]` for now.
  - In `fleet-gui`, `field.rs` shows a badge's text through `def.value_labels.get(&text)` when present.
  - Run `cargo test --workspace`. Expected: PASS, after updating GUI tests that looked for `"permission"` to `"Permission needed"`.

---

### Task 4: Remembered repos in SQLite

**Files:**
- Create: `crates/fleet-daemon/migrations/002_repos.sql`.
- Modify: `crates/fleet-daemon/src/store.rs`.
- Test: `crates/fleet-daemon/tests/store.rs`.

**Interfaces:**
- Produces (in `store.rs`):

```rust
pub struct RepoRow { pub name: String, pub source: RepoSource, pub location: String, pub base: PathBuf }
impl Store {
    pub async fn repos(&self) -> Result<Vec<RepoRow>>;                              // ordered by added_at, name
    pub async fn repo(&self, name: &str) -> Result<Option<RepoRow>>;
    pub async fn repo_by_location(&self, location: &str) -> Result<Option<RepoRow>>;
    pub async fn insert_repo(&self, r: &RepoRow) -> Result<()>;                     // error if the name exists
    pub async fn seed_repos(&self, seed: &[crate::config::Repo]) -> Result<()>;     // path rows, skips names or locations already present
}
```

- Table: `repos(name TEXT PRIMARY KEY, source TEXT NOT NULL, location TEXT NOT NULL UNIQUE, base TEXT NOT NULL, added_at INTEGER NOT NULL)`, created with `CREATE TABLE IF NOT EXISTS`. Run 002 after 001 in `open_with`.
- In `main.rs`, call `store.seed_repos(&cfg.seed_repos)` after `Store::open`.

- [ ] **Step 1: Failing tests:**
  - **`insert_then_list_and_lookup_by_name_and_location`.**
  - **`inserting_a_taken_name_fails`.**
  - **`seeding_twice_adds_nothing_the_second_time`.**
  - **`seeding_skips_a_location_already_remembered_under_another_name`.**
- [ ] **Step 2: Run** `cargo test -p fleet-daemon --test store`. Expected: FAIL.
- [ ] **Step 3: Implement.** Writes take `write_lock`, as the existing writes do.
- [ ] **Step 4: Run** `cargo test -p fleet-daemon`. Expected: PASS.

---

### Task 5: Repo resolution, base clones and worktrees (`repos.rs`)

**Files:**
- Create: `crates/fleet-daemon/src/repos.rs`; export it in `lib.rs`.
- Modify: `crates/fleet-daemon/src/git.rs`.
- Test: `crates/fleet-daemon/tests/repos.rs`. Use local bare repos and `file://` URLs; no network.

**Interfaces:**
- Produces:

```rust
pub enum Source { Name(String), Path(PathBuf), Url(String) }
pub fn classify(input: &str, home: &Path) -> Result<Source, RepoError>;      // pure; resolution order from Global Constraints (Name only if remembered: see resolve)
pub struct Repos { store: Arc<Store>, state_dir: PathBuf, projects_root: PathBuf, home: PathBuf, clone_lock: tokio::sync::Mutex<()> }
impl Repos {
    pub fn new(store: Arc<Store>, state_dir: PathBuf, projects_root: PathBuf, home: PathBuf) -> Repos;
    /// Remembered name, else path, else URL; clones a new URL into `<state_dir>/repos/<name>`; remembers new sources.
    pub async fn resolve(&self, input: &str) -> Result<RepoRow, RepoError>;
    /// fetch (URL always; path only with an `origin` remote; failure = warning string), then worktree at `dest`
    /// on `branch` from `origin/HEAD` if it exists else HEAD.
    pub async fn attach(&self, repo: &RepoRow, dest: &Path, branch: &str) -> Result<(Worktree, Option<String>), RepoError>;
}
#[derive(Debug, thiserror::Error)] pub enum RepoError {
    Unrecognised(String),            // names the accepted forms
    NotARepo(PathBuf),
    InsideProjects(PathBuf),
    Clone { url: String, stderr: String },
    Git(#[from] GitError), Store(#[from] StoreError), Io(#[from] std::io::Error) }
```

- Name derivation: the dir name (path) or the last URL segment without `.git`, made safe with the existing `is_safe_dir_name` rule (move it to `repos.rs` or make it `pub(crate)`). On a clash with a different location, append `-2`, `-3`, …
- `git.rs`:
  - `create` takes a `start: &str` ref instead of always `HEAD`. Existing callers pass `"HEAD"`.
  - Add `pub async fn fetch(repo: &Path) -> Result<(), GitError>`, `pub async fn has_remote(repo: &Path, name: &str) -> Result<bool, GitError>`, and `pub async fn default_start(repo: &Path) -> Result<String, GitError>` (`origin/HEAD` when `git rev-parse --verify origin/HEAD` works, else `HEAD`).
- A URL clone goes to a temp dir `<state_dir>/repos/.tmp-<uuid>` first and is renamed into place on success. A failure removes the temp dir and remembers nothing.
- `clone_lock` serialises resolve-of-URL, so two concurrent resolves of one URL clone once.

- [ ] **Step 1: Failing tests:**
  - **`classify_orders_path_then_url`:** `/x` and `~/x` are paths; `https://h/o/r.git`, `ssh://h/r` and `git@h:o/r.git` are URLs; `"hello world"` is `Unrecognised`.
  - **`resolve_remembers_a_path_once`:** resolving the same path twice gives one row.
  - **`resolve_by_remembered_name`.**
  - **`resolve_rejects_a_non_repo_dir`.**
  - **`resolve_rejects_a_path_inside_the_projects_root`** (Review Focus 3).
  - **`resolve_clones_a_url_once`:** a `file://` bare repo; the base is under `<state>/repos/`; a second resolve makes no new dir.
  - **`concurrent_resolves_of_one_url_clone_once`:** `tokio::join!` two resolves (Review Focus 2).
  - **`a_failed_clone_leaves_nothing`:** a `file:///nope.git` gives `RepoError::Clone` with git's stderr, no dir under `repos/` and no row (Review Focus 1).
  - **`name_clash_with_another_location_gets_a_suffix`.**
  - **`attach_starts_from_the_remote_default_branch`:** the bare repo has a commit the base clone lacks; after attach, the worktree HEAD equals the bare repo's `main`.
  - **`attach_without_origin_uses_head_and_no_warning`.**
- [ ] **Step 2: Run** `cargo test -p fleet-daemon --test repos`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p fleet-daemon`. Expected: PASS.

---

### Task 6: Projects in `AgentService`: spawn, attach, detect, kill

**Files:**
- Modify: `crates/fleet-daemon/src/agents.rs`, `crates/fleet-daemon/src/http.rs`, `crates/fleet-daemon/src/main.rs`.
- Test: `crates/fleet-daemon/tests/{agents,http}.rs`, `crates/fleet-daemon/tests/common/mod.rs` (the fixture config gets a temp `projects_root`).

**Interfaces:**
- Consumes: `Repos` (Task 5), the store repo rows (Task 4), the API types (Task 3).
- Produces (`agents.rs`):

```rust
impl AgentService {
    pub fn new(cfg: Config, store: Arc<Store>, repos: Arc<Repos>, tmux: Tmux, opts: ServiceOptions) -> AgentService;
    pub async fn spawn(&self, req: SpawnRequest) -> Result<Agent, SpawnError>;           // repos may be empty
    pub async fn attach_repo(&self, id: &AgentId, input: &str) -> Result<Workspace, ServiceError>;
    pub async fn kill(&self, id: &AgentId) -> Result<KillReport, ServiceError>;
    pub async fn adopt_owned_repos(&self, a: &Agent) -> Result<(), ServiceError>;         // called from reconcile_one for every live agent
}
```

- **Spawn:**
  - The project folder is `<projects_root>/<name>`, where `name` is the first free slug: no such folder, and no `<prefix>/<name>` branch in any of the requested repos' bases.
  - Resolve every `req.repos` entry first. Any error aborts before anything is created; return `SpawnError::Repo(RepoError)`, which maps to HTTP 400.
  - Then attach each repo at `<project>/<repo-name>` on `<prefix>/<name>`.
  - Undo on failure removes every worktree made so far plus the folder.
  - `SpawnError::UnknownRepo` goes away.
- **`CLAUDE.md`:**
  - `fn project_claude_md(workspaces: &[Workspace]) -> String` replaces the one-repo version.
  - It is written at spawn and rewritten by `attach_repo` and `adopt_owned_repos`.
  - With zero workspaces it says:
    - "This project has no repos yet."
    - "To start a new repo, create a subfolder and run `git init` in it."
    - "To work in an existing repo, call the `request_repo` tool (fleet MCP server); never clone or copy repos yourself."
  - With workspaces it lists `- \`<repo>/\` (branch \`<b>\`)` per workspace, imports each `<repo>/CLAUDE.md` that exists, and keeps the two instructions above.
- **`attach_repo`:**
  - Same-name workspace already present: return it (no-op).
  - Otherwise `Repos::resolve`, then `Repos::attach` at `<workdir>/<name>` on the project's branch (the branch of the first workspace, else `<prefix>/<folder-name>`).
  - Push the `Workspace` via `store.update_agent`, which emits `WorkspacesChanged`.
  - Rewrite `CLAUDE.md`.
  - If the session is alive, `send_literal` this line: `fleet-board: repo <name> is now available at <path> (branch <branch>).` Add the fetch warning if there was one.
- **`adopt_owned_repos`:** for each direct subdir of `workdir` with a `.git` **directory** whose name is not a workspace:
  - `Repos::resolve(<path>)`. The inside-projects-root rule must allow this one case; add `Repos::remember_owned(path) -> RepoRow`, which skips that rule.
  - Push `Workspace { owned: true, base: EMPTY_TREE, branch: <current branch or "HEAD">, .. }`.
  - Rewrite `CLAUDE.md`.
- **Kill:**
  - For non-owned workspaces, `git::remove` from the remembered repo's `base`; an unknown repo is skipped with a warning instead of `UnknownRepo`.
  - Remove the daemon's `CLAUDE.md` and the agent dir.
  - Then delete the folder only if it is empty. Otherwise `KillReport { kept: Some("kept <folder>: it contains <comma list of entries>") }`.
- **HTTP:**
  - `/api/repos` lists `store.repos()` as `RepoInfo`.
  - New `POST /api/agents/{id}/repos`.
  - `DELETE` returns `KillReport` JSON.
- **Main:** builds `Repos` with `config::home_dir()` and passes it in. `std::fs::create_dir_all(&cfg.projects_root)` at startup.

- [ ] **Step 1: Failing tests** (fake claude, temp dirs):
  - **`spawn_with_no_repos_makes_an_empty_project_folder`:** the folder is `<root>/<slug>`, `CLAUDE.md` says "no repos yet", there are no workspaces, and the tmux session runs in the folder.
  - **`spawn_with_two_repos_makes_two_worktrees_on_one_branch`.**
  - **`spawn_with_a_bad_repo_creates_nothing`:** 400; no folder, no card, no branch.
  - **`second_spawn_with_the_same_goal_gets_slug_2`.**
  - **`attach_repo_adds_a_workspace_and_tells_the_agent`:** `WorkspacesChanged` is emitted, the pane contains `repo <name> is now available`, and `CLAUDE.md` lists it.
  - **`attaching_a_repo_twice_is_a_no_op`.**
  - **`an_agent_initialised_repo_is_adopted_as_owned`:** `git init <folder>/newtool` plus a commit; after reconcile, a workspace with `owned: true` and a remembered row exist.
  - **`kill_keeps_an_owned_repo_and_reports_it`** (Review Focus 5).
  - **`kill_of_a_plain_project_removes_the_folder`.**
  - **`http_repos_lists_remembered_repos`.**
  - **`http_add_repo_returns_the_workspace`.**
- [ ] **Step 2: Run** `cargo test -p fleet-daemon`. Expected: FAIL (new tests and signature errors).
- [ ] **Step 3: Implement.** Keep the existing spawn invariants:
  - the card is stored before tmux;
  - full undo on failure;
  - the `spawning` guard and `pick_lock` around the name pick.
- [ ] **Step 4: Run** `cargo test -p fleet-daemon` and the clippy gate. Expected: PASS.

---

### Task 7: The agent asks: MCP `request_repo`, grant and refuse

**Files:**
- Create: `crates/fleet-daemon/src/mcp.rs`.
- Modify: `agents.rs` (requests and the waiter map), `http.rs` (routes), `hooks_cfg.rs` (write `mcp.json`), `ingest.rs` (keep a pending request blocked), `main.rs` (clear stale requests).
- Test: `crates/fleet-daemon/tests/mcp.rs`.

Read `docs/superpowers/findings/mcp-spike.md` first. **HOLD** vs **ASYNC** decides the tool's return.

**Interfaces:**
- Produces:

```rust
// hooks_cfg.rs
pub fn write_mcp_config(dir: &Path, base_url: &str, agent: &AgentId, token: &str) -> Result<PathBuf, SettingsError>; // 0600, http server "fleet"
// agents.rs
impl AgentService {
    pub async fn request_repo(&self, id: &AgentId, repo: &str, why: &str) -> Result<RepoAnswer, ServiceError>; // HOLD: awaits the decision
    pub async fn grant_repo(&self, id: &AgentId, input: &str) -> Result<Workspace, ServiceError>;
    pub async fn refuse_repo(&self, id: &AgentId, reason: &str) -> Result<(), ServiceError>;
    pub async fn clear_interrupted_requests(&self) -> Result<(), ServiceError>;
}
pub enum RepoAnswer { Granted(Workspace), Refused(String) }
// mcp.rs
pub async fn handle(svc: &AgentService, id: &AgentId, body: serde_json::Value) -> serde_json::Value; // JSON-RPC 2.0
```

- **MCP methods:**
  - `initialize` returns `protocolVersion` (echo the client's), `capabilities: {tools: {}}` and `serverInfo: {name: "fleet-board"}`.
  - `notifications/initialized` returns 202 with no body.
  - `tools/list` returns two tools:
    - `list_repos()`;
    - `request_repo(repo: string, why: string)`, required.
  - `tools/call` returns `{content: [{type: "text", text}], isError}`.
  - An unknown method is error `-32601`.
- **Route:** `POST /internal/mcp/{id}`, token-checked like hooks, `application/json` responses.
- **`request_repo`:**
  - The project already has the repo: answer at once with its path.
  - A request is already pending: `isError` "a repo request is already waiting for an answer".
  - Otherwise:
    1. Set `repo_request = Some({repo, why})`, status NeedsYou, `blocked_reason = RepoAccess`, `activity = "wants repo <repo>"`.
    2. Register a `oneshot::Sender<RepoAnswer>` in `Mutex<HashMap<String, Sender>>` on the service.
    3. **HOLD:** await it.
    4. Grant text: `granted: <name> at <path> (branch <b>)`. Refusal text: `refused: <reason or "no reason given">`.
  - **ASYNC:** return `requested <repo>; wait — the answer will arrive in this session` at once, and the grant or refusal is typed into the session by `grant_repo` / `refuse_repo`.
- **Grant:**
  - `attach_repo(id, input)`.
  - Clear `repo_request`, set status Working and `blocked_reason` None.
  - Send `RepoAnswer::Granted` (or type it in ASYNC).
  - With no pending request: 409.
- **Refuse:** clear the request, set status Working, send `Refused(reason)`.
- **Ingest:** while `repo_request.is_some()`, a hook may not change `status` or `blocked_reason`, though it may update `activity`, so the card stays in Needs you.
- **Spawn:** `write_mcp_config` beside `settings.json`; argv adds `--mcp-config <path>` before `--`.
- **Startup:** `clear_interrupted_requests` runs after `reconcile`. Each stored `repo_request` is cleared, with `activity = "repo request interrupted by daemon restart"`, `blocked_reason` None and status Working.

- [ ] **Step 1: Failing tests:**
  - **`mcp_rejects_a_bad_token`.**
  - **`tools_list_names_both_tools`.**
  - **`request_for_a_repo_the_project_has_answers_at_once`.**
  - **`request_puts_the_card_in_needs_you_with_repo_access`:** spawn the request call as a task; assert the store state.
  - **`grant_attaches_and_answers_the_held_call`:** the task's response text starts with `granted:`, the workspace exists, and the status is Working (HOLD).
  - **`grant_with_an_edited_target_attaches_the_edited_repo`.**
  - **`refuse_answers_with_the_reason`.**
  - **`a_second_request_while_one_is_pending_is_an_error`.**
  - **`hooks_do_not_move_a_card_off_a_pending_request`:** ingest `PreToolUse` and `Stop` while pending; the status stays NeedsYou + RepoAccess.
  - **`startup_clears_an_interrupted_request`** (Review Focus 4).
  - **`spawn_passes_mcp_config_and_the_file_is_private`:** the fake claude's recorded argv contains `--mcp-config`, and the file mode is `0o600`.
- [ ] **Step 2: Run** `cargo test -p fleet-daemon --test mcp`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p fleet-daemon` and the clippy gate. Expected: PASS.

---

### Task 8: Client: calls and config for projects

**Files:**
- Modify: `crates/fleet-client/src/http.rs`.
- Test: `crates/fleet-client/tests/` (the existing http test file; it runs against a real daemon in `client_e2e.rs`, so add there).

**Interfaces:**
- Produces:

```rust
impl FleetClient {
    pub fn spawn(&self, repos: &[String], prompt: &str) -> Result<Agent, FleetError>;
    pub fn repos(&self) -> Result<Vec<RepoInfo>, FleetError>;
    pub fn add_repo(&self, id: &str, repo: &str) -> Result<Workspace, FleetError>;
    pub fn grant_repo(&self, id: &str, repo: &str) -> Result<Workspace, FleetError>;
    pub fn refuse_repo(&self, id: &str, reason: &str) -> Result<(), FleetError>;
    pub fn kill(&self, id: &str) -> Result<KillReport, FleetError>;   // if kill already exists, change its return
}
```

- [ ] **Step 1: Failing e2e tests** (in `crates/fleet-daemon/tests/client_e2e.rs`):
  - **`client_spawns_a_project_with_no_repos`.**
  - **`client_adds_and_lists_repos`.**
  - **`client_grant_and_refuse_round_trip`:** drive a request through `/internal/mcp` with the agent token from the store.
- [ ] **Step 2: Run** `cargo test -p fleet-daemon --test client_e2e`. Expected: FAIL.
- [ ] **Step 3: Implement.**
  - The GUI call sites `app.rs` `spawn` / `repos` change type. Make them compile:
    - `AppResult::Repos` carries `Vec<RepoInfo>`;
    - the GUI keeps `repos: Vec<RepoInfo>`;
    - the existing top-bar filter uses the names.
  - Kill's `KillReport.kept` sets `self.problem`. The schema's kill action runs through `client.run`, so in `run` read the body when the action id is `kill` and return it. Simplest: `run` returns `Result<Option<String>, FleetError>`, carrying `kept`, and `AppResult::Action(Ok(Some(msg)))` sets `problem`.
- [ ] **Step 4: Run** `cargo test --workspace`. Expected: PASS.

---

### Task 9: GUI first-run screen

**Files:**
- Modify: `crates/fleet-gui/src/shell.rs`, `crates/fleet-gui/src/main.rs`.
- Test: `crates/fleet-gui/tests/shell.rs`.

**Interfaces:**
- Produces:
  - `Stage::FirstRun { root: String, error: Option<String> }` in `Root`.
  - `Root::new` takes `config_path: PathBuf` and `home: PathBuf`, and starts in `FirstRun` when `!config::exists(&config_path)`.
  - `pub fn Root::first_run(...)` (test constructor), mirroring `Root::running`.
- **The screen:** use `components::centred_card`.
  - Heading: "Welcome to fleet-board".
  - Body: "Where should projects live? Each project gets its own folder here."
  - A `TextEdit` labelled "Projects folder", prefilled `~/fleet-projects`.
  - Mono `text_low` hint: "Trust it once for Claude: cd <root> && claude, accept, /exit", with a Copy ghost button (`ui.ctx().copy_text`).
  - Primary "Continue". The error, if any, in `danger`.
- **On Continue:**
  1. Expand `~`.
  2. `create_dir_all`.
  3. `config::write_initial`.
  4. Then the existing `Starting` stage (launch the daemon).
  - An I/O error stays on the screen with the message.

- [ ] **Step 1: Failing tests:**
  - **`missing_config_shows_first_run_with_the_default_root`.**
  - **`continue_writes_the_config_and_creates_the_folder_then_starts`:** temp home. After the click, the config exists and contains `projects_root`, the folder exists, and the stage is `Starting` (use the `EnsureFn` test double, as existing tests do).
  - **`an_unwritable_root_shows_the_error_and_stays`.**
  - **`existing_config_skips_first_run`.**
- [ ] **Step 2: Run** `cargo test -p fleet-gui --test shell`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p fleet-gui` and the clippy gate. Expected: PASS.

---

### Task 10: GUI New project dialog with a repo picker

**Files:**
- Create: `crates/fleet-gui/src/components/repo_picker.rs`.
- Modify: `crates/fleet-gui/src/components/{new_agent,mod}.rs`, `crates/fleet-gui/src/app.rs`.
- Test: `crates/fleet-gui/tests/panels.rs`, `crates/fleet-gui/tests/app.rs`.

**Interfaces:**
- Produces:

```rust
// repo_picker.rs: shared by the dialog (Task 10) and the panel's Add repo (Task 11)
pub struct PickerState { pub chosen: Vec<String>, pub typed: String }
/// Remembered repos as toggle chips plus a "path or git URL" field with Add; returns true when `chosen` changed.
pub fn repo_picker(ui: &mut Ui, id_salt: &str, remembered: &[RepoInfo], st: &mut PickerState) -> bool;
// new_agent.rs
pub struct SpawnState { /* existing */ pub picker: PickerState }
pub fn new_agent_dialog(ctx: &egui::Context, repos: &[RepoInfo], st: &mut SpawnState) -> Option<(Vec<String>, String)>;
```

- **Copy:**
  - Title: "New project".
  - Goal field caption: "Goal".
  - "Repos (optional)" caption over the picker.
  - Primary button: "Start project".
  - Top-bar button: "New project".
- **The picker:**
  - Chips for remembered repos, toggled by click, with the selected chip in the `raised` fill.
  - Typed entries appear as chips with ✕ to remove.
  - Add (or Enter in the field) appends the trimmed text if it is non-empty and not a duplicate.
- **Dialog behaviour:** Ctrl+Enter starts. Every close route keeps the text, as now.

- [ ] **Step 1: Failing tests:**
  - **`starting_with_no_repos_sends_an_empty_list`.**
  - **`picking_a_remembered_repo_and_typing_a_url_sends_both_in_order`.**
  - **`typing_the_same_entry_twice_adds_it_once`.**
  - **`removing_a_typed_chip`.**
  - Update the existing tests for "New project" / "Start project" / "Goal".
- [ ] **Step 2: Run** `cargo test -p fleet-gui --test panels --test app`. Expected: FAIL.
- [ ] **Step 3: Implement.** Spawn calls `client.spawn(&repos, &prompt)`.
- [ ] **Step 4: Run** `cargo test -p fleet-gui` and the gates. Expected: PASS.

---

### Task 11: GUI side panel — repos row, Add repo, repo request box

**Files:**
- Create: `crates/fleet-gui/src/components/repo_request.rs`.
- Modify: `crates/fleet-gui/src/components/preview.rs`, `crates/fleet-gui/src/app.rs`.
- Test: `crates/fleet-gui/tests/{panels,app}.rs`.

**Interfaces:**
- Produces:

```rust
pub enum PreviewAction { None, LoadDiff, Close, Reply(String), AddRepo(String), Grant(String), Refuse(String) }
pub fn repo_request_box(ui: &mut Ui, req: &RepoRequest, target: &mut String, reason: &mut String) -> Option<RequestDecision>;
pub enum RequestDecision { Grant(String), Refuse(String) }
```

- **Panel:**
  - Under the title, a **Repos** row: one neutral badge per workspace (hover shows the branch; an owned repo's tooltip says "created in this project"), then an "Add repo" ghost button.
  - The button opens an inline `repo_picker` with an Add primary button that emits `AddRepo` per chosen entry.
  - **When `agent.repo_request` is `Some`,** the bottom box is `repo_request_box` instead of the reply box:
    - "Wants `<repo>`" in Medium, then `why` in `text`;
    - a "Grant access to" field prefilled with `repo` (kept per agent in `app.rs`);
    - Primary "Grant", with Ctrl+Enter granting;
    - a "Reason (optional)" field;
    - QuietDanger "Refuse".
- **`app.rs`:** maps `AddRepo` / `Grant` / `Refuse` to `client.add_repo` / `grant_repo` / `refuse_repo` in the background. Errors go to `problem`. On success, the `WorkspacesChanged` and `StatusChanged` events update the board.

- [ ] **Step 1: Failing tests:**
  - **`a_pending_request_replaces_the_reply_box`:** no "Answer…" field; "Wants portal" and Grant are present.
  - **`grant_sends_the_edited_target`.**
  - **`refuse_sends_the_reason`.**
  - **`the_repos_row_lists_workspaces_and_owned_ones_say_so`.**
  - **`add_repo_emits_one_action_per_chosen_entry`.**
  - **`a_repo_access_card_shows_the_repo_access_badge`** (board, through `value_labels`).
- [ ] **Step 2: Run** `cargo test -p fleet-gui --test panels --test app --test board`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p fleet-gui` and the gates. Expected: PASS.

---

### Task 12: Docs, installer and a real run

**Files:**
- Modify: `packaging/install.sh`, `docs/acceptance-gui.md`, `docs/ui.md` (only if a rule changed), `TODO.md`.
- Create: `docs/superpowers/findings/projects-first.md`.

**`TODO.md` has someone else's uncommitted edits.** Only tick or append the two items below, and keep every other line byte-identical.

- [ ] **Step 1: Installer.**
  - Stop writing `config.toml`; the first launch does it.
  - The printed trust step names `~/fleet-projects` (or the chosen root) instead of `<state>/worktrees`.
- [ ] **Step 2: `docs/acceptance-gui.md`.** Replace the config and trust bullets with the first-run flow. Add checks for:
  - a project with no repos;
  - a project with a path repo and a URL repo;
  - a manual Add repo;
  - an agent `request_repo` granted with an edited target;
  - a refusal;
  - Kill keeping an owned repo.
- [ ] **Step 3: `TODO.md`.**
  - Tick "Let an agent add a repo mid-session".
  - Append `- [ ] Add a settings page (projects root, claude command) — the first-run screen sets the root once (2026-10-24)`.
- [ ] **Step 4: Real run.**
  - Use a temp `HOME` and the real `claude`, with Antonio's login, which is his choice for this run.
  - Run through a sandbox root.
  - Confirm on the real CLI:
    - `--mcp-config` reaches the daemon;
    - `request_repo` shows the card;
    - Grant returns to the agent.
  - Capture board and panel screenshots on Xvfb as in `egui-restyle.md`.
  - Write `projects-first.md` with what passed and what didn't.
- [ ] **Step 5: Final gates.** Run `cargo test --workspace`, clippy, and `./build.sh`'s Windows check line. Expected: PASS.
