# Folder picker and continue-from-checkout: design

Date: 2026-10-24. Scope: `fleet-gui`, `fleet-daemon`, a new shared crate. The web UI stays frozen.
Builds on `2026-10-24-projects-first-design.md` (commit 137c19d).

## Why

Today every repo or folder is typed or pasted. Antonio's repos already live in folders such as
`/mnt/users/afrontino/dev`, often with work in progress on a feature branch or in a worktree. Two problems:

1. Typing paths is slow and error-prone. He wants to pick folders.
2. A picked local repo starts the agent from `origin/HEAD`, so unpushed commits and the branch he was on are
   ignored. Nobody can hand part of ongoing work to an agent; they would have to finish and push first.
   "Continue from the last local commit for the specific worktree or branch is important, otherwise people cannot
   use this app until what they're working on is done."

## Goals

- **A Browse button next to every text box that takes a folder or repo.** It opens a modal picker on top of the
  window. Select fills the text box next to the button and closes the modal. The text box stays editable; typing
  or pasting still works, and the box's content is the only thing that is submitted.
  - New project dialog: the "path or git URL" field (repo mode).
  - Side panel, Add repos: the same field (repo mode).
  - Repo request box: "Grant access to" (repo mode).
  - First-run screen: "Where should projects live?" (folder mode). The future settings page reuses it.
- **The picker understands repos.**
  - A path bar at the top (editable; Enter navigates there), Up and Home buttons, then the subfolders of the
    current folder.
  - A folder that is a git checkout (main checkout or linked worktree) shows a badge with its current branch, and
    a dirty marker when it has uncommitted files.
    - **Amended 2026-10-24 after the real run:** folder rows show the checkout's branch badge only. The dirty
      count shows on the checkout rows after expanding a repo; the per-row dirty marker is deferred.
  - Selecting a repo row in repo mode expands its **checkouts**: the main checkout and every linked worktree
    (`git worktree list`), each with its path, branch (or "detached"), last commit (short hash and subject) and
    "N uncommitted files won't be included" when dirty. Worktrees under the projects root (agents' copies) are not
    listed.
  - Choosing a checkout and pressing Select puts **that checkout's path** in the text box.
  - Folder mode: Select takes the current folder. Repo mode: Select is enabled only when a checkout is chosen.
  - Folders under the projects root are shown greyed out and are not selectable in repo mode (they are agents'
    copies).
    - **Amended 2026-10-24 after the real run:** the picker greys them, but the daemon does not reject an agent's
      copy with `InsideProjects`. A typed path inside an agent's worktree is mapped to its main repo, and the
      project starts from that worktree's HEAD.
  - Hidden folders (dot-prefixed) are not listed.
- **Continue from the picked checkout.**
  - The repo is still remembered once, by its main checkout (as today: `resolve_path` maps any checkout or
    subfolder to the main root), so the same repo is never cloned or remembered twice.
  - The project's worktree starts from the **HEAD commit of the checkout that was given** (a linked worktree's path
    means that worktree; a subfolder means its containing checkout), on the new branch `fleet/<slug>` as today.
    Unpushed commits are included. Uncommitted changes are not.
  - A remembered name (a chip, or a name typed or requested by the agent) means the **main checkout's HEAD**.
  - A URL repo (fleet-board's own clone) keeps starting from `origin/HEAD`, after a fetch, as today.
  - A path repo is no longer fetched on attach: its start point is local, so the fetch bought nothing and cost a
    network round trip.
  - The user's branch is never checked out twice and never moved.
- **Explain merging back, once in the app and once in the docs.**
  - The worktree records which branch it started from. Hovering a repo chip in the side panel shows
    "fleet/<slug>, started from <branch> at <short hash>" and a short note: merge fleet/<slug> into <branch> first
    (a fast-forward if you did not commit there meanwhile), then merge <branch> as usual. The checkout rows in the
    picker carry the same note as a tooltip.
  - A short "Working with repos" section in a new `docs/usage.md` says the same in prose: the agent's branch
    contains all of the starting branch's commits; merging it makes the old branch fully merged; if you kept
    committing on the old branch, merge fleet/<slug> into it first.

## Non-goals

- Picking files. Only folders.
- The native GNOME/portal file dialog (decided: in-app, because it can show repos and worktrees).
- Carrying uncommitted changes into the project (would need stash or copy; out of scope).
- Listing branches that have no checkout. Only checkouts are offered.
- Browsing remote hosts or git URLs.
- The settings page itself.

## Anti-goals

- **Moving, checking out or committing anything in the user's repos.** The picker only reads. Attaching only adds
  a branch and a worktree entry, as today.
- **Two implementations of folder or repo scanning.** One crate, used by every caller.
- **A UI freeze on slow disks.** `/mnt/users` is a network home; listing and `git status` can be slow, so the
  picker never blocks the frame.

## Decided

### Where the logic lives
- A new workspace crate, `fleet-fs` (sync, std only, runs the `git` CLI): `list(dir) -> Listing` (subfolders,
  each marked plain / checkout with branch) and `checkouts(path) -> Vec<Checkout>` (main + linked worktrees with
  branch, head hash, subject, dirty count). The GUI calls it directly on a background thread. Claude's call: the
  first-run screen runs before the daemon exists, and the GUI and daemon always share a machine (loopback only),
  so a daemon endpoint would not cover every caller.
- `fleet-fs` also owns the read-only repo queries the daemon needs: `main_root` (moved from `git.rs`), the
  containing checkout of a path, its HEAD and current branch. The daemon calls them through `spawn_blocking`, so
  each query exists once. `git.rs` keeps what changes things: worktree add/remove, clone, fetch, diff. Claude's call.
- "Dirty" is `git status --porcelain --ignore-submodules=dirty`: tracked changes plus untracked files, ignored
  files excluded. The count is its number of lines. Claude's call.
- The projects root to grey out comes from the GUI's client config (`projects_root`), compared after
  canonicalising both paths, as `resolve_path` does. `fleet-fs::list` takes it as an `exclude` argument. On the
  first-run screen there is no root yet, and folder mode greys nothing. Claude's call.

### Start point
- `Repos::resolve` returns the remembered row plus a start: `MainHead` (a remembered name, which is looked up
  first as today), `Checkout(path)` (a path: the canonical containing checkout of what was typed or picked), or
  `OriginHead` (a URL). `attach` takes that start instead of calling `default_start`. Spawn, Add and Grant all go
  through it. Claude's call.
- If the start cannot be read at attach time (the checkout was deleted or moved, its HEAD is unborn, or the main
  checkout is on an unborn branch), the attach fails with a clear error naming the path, and nothing is created.
  There is no fallback to another commit: a silent fallback would start the agent from work the user did not pick.
  Claude's call.
- `from_branch` is recorded the same way for every new workspace (spawn, Add, Grant); existing rows read as None and
  the tooltip then shows only the start commit. The change reaches the frozen web UI only as regenerated ts-rs
  types, which must still build.

### Data
- `Workspace` gains `from_branch: Option<String>` (`#[serde(default)]`, ts optional): the branch the start commit
  was on, None for detached or for owned repos. `base` keeps holding the start commit, so Diff and `has_changes`
  are unchanged.
- `SpawnRequest`, `AddRepoRequest`, `GrantRepoRequest` are unchanged: still strings. A path string now also
  carries the checkout choice.

### Picker opening place
- Opens at the text box's content when it is an existing folder, else the last folder the picker selected from
  (saved in `gui.toml`), else the home folder. Claude's call.

## Left to the builder

- The picker's layout details, keyboard navigation (arrows, Enter to open, Backspace for Up), using existing
  widgets and tokens.
- How the background scan is cancelled when the user navigates away.
- How much of `checkouts` is computed per row versus on expand (dirty counts are the slow part).
- The exact tooltip wording, within the meaning above.
