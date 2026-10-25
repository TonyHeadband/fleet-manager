# Projects view and project lifecycle (Stop, Archive, passport)

Written 2026-10-24 with Antonio. Started from #104 (sweep orphan worktrees); it grew into a project lifecycle and a
Projects view. Mockups: `.superpowers/brainstorm/2795416-1792895589/content/projects-view-v3.html` and
`kanban-counts.html` (git-ignored; the descriptions below are the reference).

## Why

The scarce resource is attention, and disk is the silent cost of running agents: every project holds a worktree per
repo, and each worktree grows its own build output (the two worktrees removed on 2026-10-24 held 7.6 GB and 22 GB).
Today Kill deletes everything at once, and a project then vanishes from the database, so there is no record of what
ran, what it cost or what it left behind. A crash, a manual delete or a failed Kill leaves folders and worktrees no
one owns, and nothing reports them.

Antonio wants one place that shows every project like a passport: what it is, what it holds on disk, how many agents
it used, how long it ran, what went wrong, and a deliberate way to clean it up. Nothing that ever ran is forgotten.

## Goals

- Split Kill in two: **Stop** ends the agent; **Archive** cleans up the disk later, deliberately.
- A **Projects** view (layout below) that lists every project with its state, size and history.
- Project records **never leave the database**. Archiving marks a record archived; it is never deleted.
- **Orphans** (project folders, worktrees, daemon files or tmux sessions with no project record) are found at daemon
  startup and shown in the Projects view, where they can be archived like a project.
- Project counts by state visible **from the Kanban board** at a glance.

## Anti-goals (a failure even if it works)

- Deleting anything without the user seeing exactly what goes and confirming. A folder may hold the user's own work.
- Deleting a branch. `fleet/<slug>` branches always survive, as they do today.
- Touching anything outside the project's own folder (the path on its record, wherever the projects root was when
  it started) and the daemon's own files in `state_dir`. [my call: today's Kill already acts on the recorded
  folder; the projects root can change in Settings, and a project started under the old root is still the
  daemon's]
- Deleting a project record.

## Non-goals for this version

- Tokens and cost: they need transcript parsing (#57). The view shows them as "not tracked yet".
- Resuming a stopped project (#113). Stopped is where Resume will live later.
- Renaming projects (rest of #103), multi-agent projects (orchestrator), a Projects view in the web UI (#75).
- Partial cleanup of a live project (for example deleting only `target/`). [my call: keeps Archive the only
  destructive action; can come later]

## Lifecycle (Antonio's)

```
Running ──Stop / agent exits / session dies──▶ Stopped (cause: Stopped, Exited or Crashed) ──Archive──▶ Archived
```

- **Running**: today's agent. On the Kanban board in its status column, as now (Queued unchanged: the seconds between
  New project and the agent's first sign of life).
- **Stopped** is one state with a **cause**, shown as a badge on the card and in the Projects view:
  - **Stopped** (amber): you pressed Stop. Stop replaces Kill everywhere (card, panel, API, UI schema, docs): it saves
    the agent's last terminal screen to the record, then ends the tmux session and any held `request_repo` call, as
    Kill does. Nothing on disk changes: worktrees, project folder, `CLAUDE.md`, `PROMPT.md`, the agent's settings
    folder and the branches all stay.
  - **Exited** (blue): the agent ended on its own (the `SessionEnd` hook: `/exit` in its terminal, a normal end,
    "No, exit" at the trust dialog; `crates/fleet-daemon/src/status.rs` maps it to `Dead` today). Log line "agent
    exited". [my call: a deliberate exit is not a crash; the hook is the signal that tells them apart] No last screen
    (the session is gone by then).
  - **Crashed** (red): the tmux session disappeared with neither a Stop nor a `SessionEnd` hook (today's `Dead` from
    `reconcile`, "session lost"). Log line with the last activity. No last screen.
- Stopped projects **stay on the board**, in the Stopped column (below). Today's `Dead` cards in the Review column
  move there.
- **Archive** (only for a Stopped project, whatever its cause; **only from the Projects view**, Antonio's: it is the one
  destructive action and that is where the full passport and the plan sit): the cleanup. It first shows a plan of what will be
  removed with sizes, per item: each worktree (with "N uncommitted files will be lost" when the worktree is dirty
  [my call: the one thing Archive can lose that the branch does not keep]), the project folder, the daemon's files.
  On confirm it removes them, using today's Kill logic (worktree removal through git with the by-hand fallback
  inside the project folder, then the folder only if empty, else "kept <path>: it contains …"), and writes the record
  one last time: final footprint, what was removed, bytes freed, anything kept. The project becomes **Archived**:
  a read-only passport, and it leaves the board.
- **Orphan**: found, not created by the lifecycle. Archiving an orphan uses the same plan, confirm and removal, and
  then creates an archived record for it, so it stays in the history. [my call]

Existing `Dead` agents on the first upgrade: "session ended" becomes Stopped/Exited, "session lost" becomes
Stopped/Crashed.
[my call, from the activity text each one already carries]

Hooks or MCP calls arriving with a stopped or archived project's token are refused, as for an unknown agent
today. [my call]

## The Projects view (Antonio's layout, details mine where marked)

A new drawer entry, **Projects**, between Kanban and Settings, in the app's existing look (header and footer rows
shared with the drawer, same fonts, colours and widgets as Settings).

- **Header**: "Projects", and a "Show archived" toggle, off by default. [my call]
- **Totals row** across the top: projects by state (running, stopped with crashed counted on its own, orphans); disk
  held by all projects and how much archiving the stopped and orphan ones would free; the clones cache under `<state_dir>/repos`;
  agents started since this version; tokens and cost as "not tracked yet".
- **Left: the project list, narrow** (Antonio: smaller than the right; the mockup uses 250 px of 1400). One row per
  project: name, state badge, size. Running projects use their board status colour; stopped ones their cause colour
  (Stopped amber, Exited blue, Crashed red); Orphan ⚠ amber; Archived dimmed. Order: running, stopped (crashed
  first), orphans, archived; newest first within each.
  [my call]
- **Right: the passport, wide**, for the selected project:
  - name, badge, started time, project folder, "Open on board" unless archived;
  - goal (the prompt's first line, as the card's task);
  - repos: each repo with its branch and the commit it started from;
  - **Footprint**: size per worktree with its `target/` (or other build output) on its own line, the daemon's files,
    the total, and "measured <when>" with a Refresh;
  - **Activity**: agents used (1 in this version); time running (start to stop) and time per status, counted from
    this version on [my call: the daemon already sees every status change, so it accumulates per-status time; no
    history before this version]; tokens and cost "not tracked yet";
  - **Log**: one line per lifecycle event with its time: started (with repos), repo added, repo granted or refused,
    daemon restarted with status unverified, stopped, crashed (with the last activity), archived (with what was
    removed, bytes freed, anything kept);
  - **Diff** (stopped projects, Antonio's; also in the board's side panel for a stopped card): each repo's changes, the same diff view and Refresh as the board's
    side panel; the worktrees are still on disk until Archive, so the daemon's diff must accept a project that is
    not running. Archived projects show no diff (their worktrees are gone);
  - **Last screen** (Antonio's): the terminal text saved at Stop, read-only, as the panel's Terminal tab shows it;
    "not saved" for Exited and Crashed; also the side panel's Terminal tab for a stopped card;
  - **Actions**: Stop when running; Archive… when stopped (disabled with "stop it first" otherwise).

## The Kanban board (Antonio's)

Mockup: `kanban-stopped-column.html`.

- **Columns**: **Stopped**, Queued, Working, Needs you, Review. The **Talking** column is removed: nothing sets the
  `Talking` status yet (it was reserved for agent-to-agent messaging, #44). The status itself stays in the API, so
  that work can bring a column back.
- **The Stopped column** comes first, on a slightly darker background [my call]. Its cards show the cause badge
  (Stopped, Exited, Crashed), the "Has changes" badge when there is work to review, a one-line reason ("stopped by
  you", "agent exited", the last activity for a crash), the size, and **Details** [my call], which opens the Projects view on that project. The card has no Archive. Clicking one opens the side panel
  with its Diff and saved last screen; the reply box is not shown.
- Running cards' **Kill** becomes **Stop**.
- **Every card has the same fixed size** (Antonio's), in every column, whatever its content. The card's slots
  are fixed: time in status; title, at most two lines; one badge row; one reason line; one action row. Text that
  does not fit is cut with an ellipsis and shown whole in a tooltip; badges that do not fit collapse into "+N".
  [my call on the slots and the overflow rule; today a card grows with its text]
- **Header counts** (option A), to the right of the Repo filter: "Projects: ● N running ● N stopped [N crashed] ⚠ N
  orphan · <total disk>". Crashed and orphan appear only when non-zero. Each count opens the Projects view filtered
  to that state.

## Footer messages (Antonio's)

The app's footer message line (the one that already carries errors and "Settings saved", with History) gets:

- a **warning** when a project crashes: "<project> crashed: <last activity>", clicking it selects that card; with "Keep errors and warnings in the footer until dismissed" on, it stays until dismissed;
- a **success** line when a Stop finishes: "<project> stopped"; an info line when an agent exits on its own:
  "<project> exited";
- a **success** line when an Archive finishes: "<project> archived, <size> freed" (or "…, kept <path>" when something
  was kept).

The drawer footer's "live · N agents · M need you" line is unchanged; the counts by state live in the Kanban header.
[my call]

## Constraints

- **Disk measurement is slow** on NFS (a 10 GB `target/` takes a long time to walk). Sizes are measured in the
  background, cached on the record with the time they were measured, and never block the UI or an API call.
  Measured at startup, after Stop, on Refresh, and as part of the Archive plan. [my call]
- Air-gapped, single user, loopback API, as everything else.
- The API is used only by the egui app and the web UI. Replacing `DELETE /api/agents/{id}` is an internal change,
  not a breaking release. [my call] The web UI's Kill comes from the server's UI schema, so it becomes Stop with
  the schema; the web UI gets no Projects view (#75).
- Agents that exist when the new version first starts get a project record each (started time unknown, shown as
  such). [my call]

## Orphan detection (from #104)

At daemon startup, next to `sweep_tmp_clones` (`crates/fleet-daemon/src/repos.rs`), and on Refresh in the view:

- folders directly under the projects root with no project record;
- worktrees registered in a remembered repo that point under the projects root, with no project record (including
  ones whose folder is gone, which `git worktree prune` would clear);
- agent folders under `state_dir` with no project record;
- tmux sessions with the daemon's session prefix on its socket with no project record.

Projects whose folder lies under an older projects root (the root can be changed in Settings) keep their own path on
the record; only the current root is scanned for orphans. [my call]

**Orphan rows** [my call]: one row per project folder found under the projects root, holding the worktrees inside it
(they are grouped by path); one row per daemon agent folder and one per worktree outside any folder, since without a
record they cannot be linked to anything else. A **live orphan tmux session** (a running Claude with no record, for
example after the database was lost) is not archived: its row offers **End session**, with a confirmation that says
an agent may be working in it; once ended, any folder it leaves shows as an ordinary orphan.

## Left to the builder

Database schema and migration (a `projects` table beside `agents`, or a widened agent record), API shapes and route
names, how the Archive plan is fetched and confirmed, the GUI component split, how the background size measurement
is scheduled, test design. Follow the existing patterns: `migrations/00N_*.sql`, the server-driven UI schema in
`crates/fleet-api/src/ui.rs`, `fleet-client` for HTTP, egui views under `crates/fleet-gui/src/views/`.

## Issues this touches

Closes #104. Covers the archive part of #103. Prepares #57 (tokens and cost show here), #51 (history), #113 (Resume
from Stopped).
