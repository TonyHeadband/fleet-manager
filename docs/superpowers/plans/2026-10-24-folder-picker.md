# Folder Picker and Continue-from-Checkout Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Browse button beside every folder/repo text box opens an in-app modal that understands repos and worktrees. A picked local checkout makes the project continue from that checkout's last commit instead of `origin/HEAD`.

**Architecture:**
- A new sync workspace crate, `fleet-fs`, owns every read-only folder and repo query: listing a folder, finding a repo's main checkout, the checkout a path is in, a checkout's HEAD, its uncommitted-file count, and the repo's checkouts.
- The daemon's `Repos::resolve` returns a start point with the remembered row, and `Repos::attach` creates the worktree from it. `Workspace` records `from_branch`.
- The GUI gets one `FolderPicker` component (state + modal + Browse button) that scans on a background thread through `fleet-fs`. It is wired into the repo picker (New project, Add repos), the repo request box (Grant) and the first-run screen.

**Tech Stack:** Rust 2021, tokio (`spawn_blocking`), axum, eframe/egui 0.37, `egui_kittest`, the git CLI.

**Spec:** `docs/superpowers/specs/2026-10-24-folder-picker-design.md`. Read it before any task. It builds on `docs/superpowers/specs/2026-10-24-projects-first-design.md` (commit 137c19d).

## Global Constraints

- **No new external crates.** `fleet-fs` is a new workspace member that depends on `std` and `thiserror` only (already in the workspace).
- **Read-only.** Nothing in `fleet-fs` writes to a repo: no checkout, stash, fetch, commit or config write. It runs `git` with argv (never a shell), `GIT_TERMINAL_PROMPT=0` and stdin null, like `crates/fleet-daemon/src/git.rs`.
- **One implementation per query.** `git::main_root` and `git::MainRoot` move out of `crates/fleet-daemon/src/git.rs` into `fleet-fs`; nothing else re-implements folder listing, checkout detection, HEAD reading or dirty counting.
- **Start points (spec, "Start point"):**
  - remembered name of a `path` repo → the main checkout's HEAD;
  - remembered name of a `url` repo, or a URL → fetch, then `origin/HEAD` if it exists, else `HEAD` (unchanged);
  - a path → the HEAD of the checkout containing that path (`git rev-parse --show-toplevel` of the canonical path).
  - A path repo is never fetched.
  - No fallback: an unreadable start fails with `RepoError::Start`, and nothing is created.
- **Dirty** = the line count of `git status --porcelain --ignore-submodules=dirty`.
- **Hidden folders** (name starts with `.`) are never listed.
- **Picker copy (exact):** button `Browse…`; modal titles `Choose a folder` (folder mode) and `Choose a repo` (repo mode); buttons `Up`, `Home`, `Select`, `Cancel`; dirty note `N uncommitted files won't be included` (`1 uncommitted file won't be included` for one).
- **Merge-back note (exact), used by both tooltips:** `Merge fleet/<slug> into <branch> first (a fast-forward if you did not commit there meanwhile), then merge <branch> as usual.` Substitute the real branch names. With no `from_branch`, the note is omitted.
- **The web UI (`web/`) is frozen.** Only the ts-rs files that `cargo test -p fleet-api` regenerates under `web/src/api/generated/` may change, and `pnpm build` (in `web/`) must still pass.
- **No git commit steps.** Antonio commits by hand.
- **Gates stay green:**
  - `cargo test --workspace`;
  - `cargo clippy --workspace --all-targets -- -D warnings -A clippy::large_enum_variant`;
  - the colour-literal grep over `crates/fleet-gui/src` outside `theme.rs` returns nothing: `grep -rnE 'Color32::(from_rgb|from_gray|from_rgba|[A-Z_]{3,})' crates/fleet-gui/src | grep -v theme.rs`.
- GUI work follows `docs/ui.md`: tokens and the primitives in `crates/fleet-gui/src/widgets` only; motion only through `theme::animate`; modals use `components::modal_frame`.
- **Real runs** never touch `~/.config/fleet-board`, `~/fleet-projects` or the daemon on port 7878. Use a throwaway `HOME`, config and port.

## Review Focus

1. **The picked worktree was deleted or moved between picking and Start.** The spawn must fail with an error naming the path, and leave no project folder, branch or worktree behind. Test in Task 2.
2. **A repo whose main checkout is on an unborn branch** (`git init`, no commit), picked by name or path. Fail clearly, never start from an empty tree silently. Test in Task 2.
3. **A folder that cannot be read** (permission denied) or a path typed into the path bar that does not exist. The picker shows the error in place and keeps the previous listing. It must not panic or close. Test in Task 3.
4. **A slow network folder.** Listing `/mnt/users/...` or `git status` on a big repo must not freeze the window: the frame keeps drawing while the scan runs, and navigating away discards stale results. Test in Task 3 (a results-for-another-dir message is ignored).
5. **Two checkouts of the same repo in one New project request** (e.g. `dev/foo` and `dev/foo-feature`). Spawn keeps the first and ignores the second, as it dedups by repo name today. The second must not fail the spawn. Test in Task 2.

---

### Task 1: `fleet-fs` crate

**Files:**
- Create: `crates/fleet-fs/Cargo.toml`, `crates/fleet-fs/src/lib.rs`
- Modify: `Cargo.toml` (workspace `members`)
- Test: `crates/fleet-fs/tests/fs.rs`

**Interfaces:**
- Produces (all sync, in `fleet_fs`):

```rust
#[derive(Debug, thiserror::Error)]
pub enum FsError {
    #[error("{0} is not a git repository")] NotARepo(PathBuf),
    #[error("git {args} failed: {stderr}")] Git { args: String, stderr: String },
    #[error(transparent)] Io(#[from] std::io::Error),
}
pub enum MainRoot { Worktree(PathBuf), Bare }
pub fn main_root(path: &Path) -> Result<Option<MainRoot>, FsError>;      // moved from fleet-daemon git.rs, same semantics
pub fn toplevel(path: &Path) -> Result<Option<PathBuf>, FsError>;        // the checkout containing `path`, None if not in one
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Head { pub commit: Option<String>, pub branch: Option<String>, pub subject: String }  // commit None = unborn; branch None = detached
pub fn head(checkout: &Path) -> Result<Head, FsError>;
pub fn dirty_count(checkout: &Path) -> Result<usize, FsError>;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checkout { pub path: PathBuf, pub main: bool, pub head: Head }
pub fn checkouts(path: &Path, exclude: Option<&Path>) -> Result<Vec<Checkout>, FsError>;  // main first, then linked, by path
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryKind { Folder, Checkout { branch: Option<String> }, Excluded }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry { pub name: String, pub path: PathBuf, pub kind: EntryKind }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing { pub dir: PathBuf, pub entries: Vec<Entry> }       // entries sorted case-insensitively by name
pub fn list(dir: &Path, exclude: Option<&Path>) -> Result<Listing, FsError>;
```

- `list`: subfolders only, symlinks to folders followed, dot-names skipped. A subfolder with a `.git` dir or file is a `Checkout` whose branch is read with `head` (unborn or detached → `None`). A subfolder at or under the canonical `exclude` is `Excluded`. `dir` in the result is canonical.
- `checkouts`: from `git worktree list --porcelain` run in `path`. Skip entries marked `prunable` and paths that no longer exist. Skip entries at or under the canonical `exclude`. `main` is the first porcelain entry. A bare main is skipped.

- [ ] **Step 1: Write the failing tests** in `crates/fleet-fs/tests/fs.rs`, with temp repos made by a small `git(dir, args)` helper (like `crates/fleet-daemon/tests/common/mod.rs:34`):
  - `list_marks_checkouts_and_skips_hidden`: a dir with `a/` (plain), `repo/` (git init + one commit on `main`), `.hidden/`. Expect `entries == [Entry{name:"a",kind:Folder}, Entry{name:"repo",kind:Checkout{branch:Some("main")}}]`.
  - `list_marks_the_excluded_root`: `exclude = Some(dir.join("projects"))`, so `projects/` is `Excluded`.
  - `list_of_a_missing_dir_is_an_io_error`: `matches!(list(&dir.join("nope"), None), Err(FsError::Io(_)))`.
  - `head_reads_branch_commit_and_subject`: after commit "first", `head.branch == Some("main")`, `head.subject == "first"`, `head.commit` equals `git rev-parse HEAD`.
  - `head_of_an_unborn_repo_has_no_commit`: `git init` only gives `commit == None`, `branch == Some("main")`.
  - `head_of_a_detached_checkout_has_no_branch`.
  - `dirty_count_counts_tracked_and_untracked_not_ignored`: modify one tracked file, add one untracked, add one ignored (via `.gitignore` committed) → `2`.
  - `toplevel_of_a_subfolder_is_its_checkout` and `toplevel_of_a_linked_worktree_subfolder_is_the_worktree`.
  - `checkouts_lists_main_then_worktrees_and_skips_excluded`: repo + `git worktree add ../feat -b feat` + `git worktree add <exclude>/p1 -b p1`. Expect two entries: main (`main: true`, branch `main`) then `feat`. Works when `path` is the `feat` worktree too.
  - `checkouts_skips_a_deleted_worktree`: `rm -rf` the `feat` dir, so only main is listed.
  - `main_root_*`: move the three existing cases (subdir, linked worktree, bare) from `crates/fleet-daemon/tests/repos.rs:188-216` as unit expectations on `main_root` (keep the daemon tests too).
- [ ] **Step 2:** `cargo test -p fleet-fs` fails to compile (crate missing).
- [ ] **Step 3: Implement** the crate. A private `fn git(dir: &Path, args: &[&str]) -> Result<String, FsError>` uses `std::process::Command` with the flags in Global Constraints.
- [ ] **Step 4:** `cargo test -p fleet-fs` passes. `cargo clippy -p fleet-fs --all-targets -- -D warnings` is clean.

### Task 2: Daemon start points and `from_branch`

**Files:**
- Modify: `crates/fleet-daemon/Cargo.toml` (depend on `fleet-fs`), `crates/fleet-daemon/src/git.rs` (remove `main_root`/`MainRoot`; `Worktree` gains `from_branch`; `GitError::Fs(#[from] fleet_fs::FsError)`), `crates/fleet-daemon/src/repos.rs:135-251`, `crates/fleet-daemon/src/agents.rs` (spawn `:327-380`, `attach` `:505-535`, owned adoption `:740-750`), `crates/fleet-api/src/model.rs:17-28`
- Modify (struct literals gain `from_branch: None`): every `Workspace { … }` in `crates/*/tests/*.rs` and `crates/fleet-api/tests/contract.rs`
- Test: `crates/fleet-daemon/tests/repos.rs`, `crates/fleet-daemon/tests/agents.rs`

**Interfaces:**
- Consumes: Task 1's `fleet_fs::{main_root, MainRoot, toplevel, head, Head}`, called through `tokio::task::spawn_blocking`.
- Produces:

```rust
// fleet-api model.rs, Workspace:
/// The branch the start commit was on (None: detached, owned, or made before this field existed).
#[serde(default)] #[ts(optional = nullable)] pub from_branch: Option<String>,
// repos.rs:
pub enum Start { MainHead, Checkout(PathBuf), OriginHead }
pub struct Resolved { pub row: RepoRow, pub start: Start }
pub async fn resolve(&self, input: &str) -> Result<Resolved, RepoError>;
pub async fn attach(&self, repo: &RepoRow, start: &Start, dest: &Path, branch: &str) -> Result<(Worktree, Option<String>), RepoError>;
RepoError::Start { path: PathBuf, reason: String }  // #[error("cannot start from {path}: {reason}")]
// git.rs:
pub struct Worktree { pub path: PathBuf, pub branch: String, pub base: String, pub from_branch: Option<String> }
pub async fn create(repo: &Path, prefix: &str, slug: &str, dest: &Path, start: &str, from_branch: Option<String>) -> Result<Worktree, GitError>;
```

- `resolve`: name hit → `Start::OriginHead` for a `url` row, else `Start::MainHead`. URL → `OriginHead`. Path → `Checkout(toplevel(canonical path))`, with the row from today's `resolve_path`, which maps to the main root and remembers it once.
- `attach`: under the base lock, check `branch_exists` (unchanged). Then compute `(commit, from_branch)`:
  - `MainHead`: `head(&repo.base)`.
  - `Checkout(p)`: `head(p)`. A missing `p` gives `Start { reason: "the folder no longer exists" }`.
  - Unborn (`commit == None`) gives `Start { reason: "it has no commit yet" }`.
  - `OriginHead`: fetch (warning on failure, as today), then `default_start`; `from_branch` = `git rev-parse --abbrev-ref origin/HEAD` (e.g. `origin/main`), or the base's current branch when there is no `origin/HEAD`.
  - Pass the commit hash as `start` to `git::create`, so the worktree starts at exactly that commit.
- Every `Workspace` built from a `Worktree` copies `from_branch`; owned ones get `None`.

- [ ] **Step 1: Write the failing tests.** In `crates/fleet-daemon/tests/repos.rs`:
  - Replace `attach_starts_from_the_remote_default_branch` (`:156`) with `path_repo_starts_from_local_head_without_fetching`: a local clone of a bare remote, plus a local unpushed commit on `main`, so `wt.base == local HEAD`, `!= origin/main`, warning `None`, `from_branch == Some("main")`, and `git rev-parse origin/main` in the base is unchanged after attach (no fetch). Use a remote URL that would fail to fetch, to prove no fetch: set `origin` to a nonexistent path after cloning.
  - `url_repo_still_starts_from_origin_head`: via `resolve(url)` → `start == OriginHead` and `wt.base == origin/HEAD`.
  - `picking_a_linked_worktree_starts_from_its_head`: base on `main`, `git worktree add ../feat -b feat` plus a commit there. `resolve(feat_path)` gives `row.location == base` (remembered once), `start == Checkout(feat)`, and attach gives `base == feat HEAD`, `from_branch == Some("feat")`.
  - `picking_a_subfolder_of_a_worktree_uses_that_worktree`.
  - `remembered_name_starts_from_the_main_checkout_head`, even after the user switched the main checkout to another branch: `from_branch` is that branch.
  - `a_deleted_checkout_fails_and_leaves_nothing` (Review Focus 1): resolve `feat`, delete `feat`, attach gives `RepoError::Start`, `branch_exists(fleet/x) == false`, and `dest` does not exist.
  - `an_unborn_main_checkout_fails_clearly` (Review Focus 2): `git init` with no commit, `resolve(path)` then attach gives `RepoError::Start` with reason containing `no commit`.
  - In `crates/fleet-daemon/tests/agents.rs`: `spawn_with_two_checkouts_of_one_repo_keeps_the_first` (Review Focus 5): `repos: [base, feat]` gives one workspace, started from base HEAD. Also `spawn_records_from_branch_on_the_workspace`.
- [ ] **Step 2:** `cargo test -p fleet-daemon --test repos --test agents` fails (signatures and field missing).
- [ ] **Step 3: Implement** per Interfaces. `RepoError::Start` maps to HTTP 409 in `crates/fleet-daemon/src/http.rs`, like `BranchExists`. `remember_owned` uses `fleet_fs::main_root` through `spawn_blocking`. Update the doc comment on `attach` (it still says "Fetch … from `origin/HEAD`").
- [ ] **Step 4:** `cargo test -p fleet-api` regenerates `web/src/api/generated/Workspace.ts`. Run `cd web && pnpm build`, which must pass. Then `cargo test --workspace` and clippy are green.

### Task 3: `FolderPicker` component

**Files:**
- Create: `crates/fleet-gui/src/components/folder_picker.rs`
- Modify: `crates/fleet-gui/src/components/mod.rs` (export), `crates/fleet-gui/Cargo.toml` (depend on `fleet-fs`), `crates/fleet-client/src/config.rs:155` (`GuiSettings` gains `#[serde(default)] pub last_pick_dir: Option<PathBuf>`)
- Test: `crates/fleet-gui/tests/folder_picker.rs`

**Interfaces:**
- Consumes: Task 1's `list`, `checkouts`, `dirty_count`, `Listing`, `Entry`, `EntryKind`, `Checkout`.
- Produces:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickMode { Folder, Repo }
pub struct FolderPicker { /* private: open, mode, dir, listing, expanded repo + its checkouts, chosen, dirty counts, error, rx, settings_path, exclude, home */ }
impl FolderPicker {
    pub fn new(mode: PickMode, settings_path: PathBuf, exclude: Option<PathBuf>, home: PathBuf) -> FolderPicker;
    pub fn is_open(&self) -> bool;
}
/// The Browse… button. When the picker is open, it also draws the modal. On Select it writes the chosen path
/// into `text`, saves the folder it was chosen from as `last_pick_dir`, closes, and returns true.
pub fn browse(ui: &mut Ui, picker: &mut FolderPicker, text: &mut String) -> bool;
```

- **Opening place:** `text` (trimmed, `~` expanded) if it is an existing dir; else `last_pick_dir` from `GuiSettings::load(settings_path)` if it exists; else `home`.
- **Modal:** `egui::Modal` with `modal_frame`. Title per mode. A path bar (`TextEdit`, Enter navigates), `Up`, `Home`, then a scrollable list of entries. Clicking a `Folder` entry navigates into it. `Excluded` entries are drawn with `text_low` and, in repo mode, do nothing on click.
  - **Repo mode:** clicking a `Checkout` entry expands its checkouts (from `checkouts(entry.path, exclude)`). Each checkout row shows path, branch (or `detached`), short hash (7 chars) plus subject, and the dirty note once its count arrives. A tooltip carries the merge-back note with `<slug>` written literally as `<project>`. Clicking a row chooses it. `Select` is enabled only with a chosen checkout and fills `text` with the checkout's path.
  - **Folder mode:** `Select` fills `text` with the current `dir`. Double-clicking a folder navigates.
  - Escape and `Cancel` close without touching `text`.
- **Scanning:** every navigation sends a job to a new `std::thread` and stores its `dir` as current. Results come back on an `mpsc` channel as `enum Scan { Listed(Listing) | Failed { dir, error } | Checkouts { repo, list } | Dirty { path, count } }`, and `ctx.request_repaint()` wakes the frame. A result whose `dir` or `repo` is not the current one is dropped. `Failed` shows the error under the path bar and keeps the previous listing. Dirty counts are computed after the checkouts list is sent, one per checkout. The frame never waits on the channel (`try_recv`).
- Give the internals a test seam: `#[doc(hidden)] pub fn deliver(&mut self, scan: Scan)` with `Scan` public, so tests can inject results without threads.

- [ ] **Step 1: Write the failing kittest tests** in `crates/fleet-gui/tests/folder_picker.rs`, against a temp dir tree like Task 1's:
  - `browse_opens_at_the_text_when_it_is_a_folder`: the path bar shows that dir.
  - `browse_opens_at_last_pick_dir_then_home`.
  - `repo_mode_select_is_disabled_until_a_checkout_is_chosen`. Choosing the `feat` checkout and pressing Select sets `text` to the `feat` path, closes, and writes `last_pick_dir` to the settings file.
  - `folder_mode_select_takes_the_current_folder`.
  - `cancel_leaves_the_text_alone`.
  - `excluded_folder_is_not_selectable_in_repo_mode`.
  - `a_failed_scan_shows_the_error_and_keeps_the_listing` (Review Focus 3): deliver `Listed(A)`, then `Failed{dir: B}` after navigating to B. Expect the error text visible and A's entries still listed.
  - `a_stale_listing_is_ignored` (Review Focus 4): navigate to B, deliver `Listed(A)`, and the entries are not replaced.
  - `dirty_note_is_singular_for_one`: deliver `Dirty{count:1}` and expect `1 uncommitted file won't be included`.
- [ ] **Step 2:** `cargo test -p fleet-gui --test folder_picker` fails.
- [ ] **Step 3: Implement** per Interfaces and `docs/ui.md`. Add `folder_picker` to the component list in `docs/ui.md`.
- [ ] **Step 4:** the tests pass. Clippy and the colour grep are clean.

### Task 4: Wire Browse into every folder and repo field

**Files:**
- Modify: `crates/fleet-gui/src/components/repo_picker.rs` (`PickerState` gains `pub browse: FolderPicker`; Browse sits between the field and Add), `crates/fleet-gui/src/components/new_agent.rs`, `crates/fleet-gui/src/components/preview.rs` (`repos_row`; `PreviewState` builds its picker), `crates/fleet-gui/src/components/repo_request.rs` (a `&mut FolderPicker` parameter; Browse beside "Grant access to"), `crates/fleet-gui/src/app.rs` (`FleetApp::new` takes `settings_path: PathBuf`, `projects_root: PathBuf`, `home: PathBuf`; per-agent request pickers next to the per-agent target text), `crates/fleet-gui/src/shell.rs` (first-run root field gets a folder-mode Browse; pass the new args to `FleetApp::new`), `crates/fleet-gui/src/main.rs`
- Test: `crates/fleet-gui/tests/panels.rs`, `crates/fleet-gui/tests/shell.rs`, `crates/fleet-gui/tests/app.rs`

**Interfaces:**
- Consumes: Task 3's `FolderPicker::new`, `browse`, `PickMode`.
- Repo-mode pickers get `exclude = Some(projects_root)`. The first-run picker gets folder mode and `exclude = None`.
- Selecting in the repo picker fills `typed` only. Add, Enter or Start commits it, unchanged.

- [ ] **Step 1: Write the failing tests:**
  - `new_project_browse_fills_the_path_field` (panels or app): open the dialog, click `Browse…`, deliver a listing plus checkouts, choose one, Select. The "Path or git URL" field holds the path, and Start sends it in `repos`.
  - `add_repo_browse_fills_the_field`.
  - `grant_browse_replaces_the_target`: the target field holds the picked path, and Grant sends it.
  - `first_run_browse_fills_the_root`.
  - Existing tests that build `FleetApp::new` or `PickerState::default()` are updated to the new constructors.
- [ ] **Step 2:** they fail.
- [ ] **Step 3: Implement.** `PickerState` can no longer derive `Default` if `FolderPicker` needs args. Give it `PickerState::new(browse: FolderPicker)` and update the callers.
- [ ] **Step 4:** `cargo test -p fleet-gui` passes. Clippy and the colour grep are clean.

### Task 5: Merge-back tooltips and docs

**Files:**
- Modify: `crates/fleet-gui/src/components/preview.rs:248-251` (chip tooltip)
- Create: `docs/usage.md`
- Modify: `docs/acceptance-gui.md` (picker checks), `docs/ui.md` if Task 3 did not
- Test: `crates/fleet-gui/tests/panels.rs`

- **Chip tooltip:** `<branch>, started from <from_branch> at <base[..7]>`, a blank line, then the merge-back note. With `from_branch: None`: `<branch>, started at <base[..7]>`. Owned keeps today's `· created in this project`. Put the text in one `pub fn workspace_tip(w: &Workspace) -> String` in `preview.rs`, so tests can assert it.
- **`docs/usage.md`**, "Working with repos":
  - picking a folder or checkout and what it starts from;
  - URL repos start from the remote's main;
  - uncommitted changes are not carried over;
  - the agent works on `fleet/<project>`, which contains every commit of the branch it started from;
  - merging it makes the old branch fully merged;
  - if you kept committing on the old branch, merge `fleet/<project>` into it first;
  - Kill keeps the branch.
  - About 20 lines.

- [ ] **Step 1: Failing tests:** `workspace_tip_names_start_and_merge_route` (from_branch `feat`, base `abcdef1234`, branch `fleet/x`; expect the exact copy) and `workspace_tip_without_from_branch`.
- [ ] **Step 2:** fail. **Step 3:** implement plus docs. **Step 4:** `cargo test -p fleet-gui` passes.

### Task 6: Real-run check

**Files:**
- Create: `docs/superpowers/findings/folder-picker.md` plus PNGs in `docs/superpowers/findings/folder-picker/`

Use a throwaway `HOME` with a config on a free port (never 7878), the real release binaries, and Xvfb or the desktop as in `docs/superpowers/findings/projects-first.md`. Inside the throwaway home:
- `dev/foo`: a clone of a local bare remote, on `main`, with one unpushed commit.
- `dev/foo-feat`: a worktree of `foo` on `feat`, with one commit plus an uncommitted file.

Record, with screenshots:
- [ ] (a) The first-run Browse in folder mode picks the projects root.
- [ ] (b) The New project Browse shows `foo` with its branch badge. Expanding it lists `foo` (`main`) and `foo-feat` (`feat`, with "1 uncommitted file won't be included"). `fleet-projects` is greyed out.
- [ ] (c) Picking `foo-feat` and starting a project: the worktree's `git log -1` equals `foo-feat`'s HEAD; the uncommitted file is absent; `foo`'s checkout, its branch and `origin/main` are unchanged.
- [ ] (d) The chip tooltip shows `started from feat` and the merge-back note.
- [ ] (e) Picking `foo` by its remembered chip starts from `foo`'s unpushed commit.
- [ ] Run the gates one last time: `cargo test --workspace`, clippy, the colour grep, `cd web && pnpm build`.
