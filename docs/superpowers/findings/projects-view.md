# Projects view acceptance (2026-10-25)

Run on the Rocky 8 workstation against `feat/projects-view` at `575c292` (the header fix below included; debug build:
`cargo build -p fleet-daemon -p fleet-gui`, binaries copied out of the shared target), with the window on a virtual X
display (`Xvfb :97 -screen 0 1400x800x24 +extension GLX`, software GL). Screenshots are in `projects-view/`. Spec:
`specs/2026-10-24-projects-view-design.md` (Lifecycle, The Kanban board, Footer messages, Orphan detection).

## How it was run

- **Isolation:** `HOME` was a scratch folder under the worktree's `.superpowers/t10-home/` (git-ignored) with its own
  `config.toml` (`listen = "127.0.0.1:7963"`, `projects_root` inside the scratch home, `claude_command` =
  `crates/fleet-daemon/tests/bin/claude`) and `.gitconfig`; tmux on its own socket (`FLEET_TMUX_SOCKET=fbt10`). Every
  process ran under `env -i` with only that `HOME`, `PATH`, `DISPLAY=:97` and the socket, so no `XDG_*` variable could
  point at the real config. Antonio's config, state, daemon on 7878 and tmux sessions were not touched.
- **Repo:** a one-commit `git init` repo under the scratch home (`repos/demo`); projects were spawned with
  `POST /api/agents` naming it.
- **No real Claude:** agents ran the test double, so a running card sits in Needs you at "Startup prompt".
- **Clicks:** a Python `ctypes` script driving `libXtst` (XTEST pointer events), kept under `.superpowers/`.
- **Daemon from a terminal:** started by hand before the app under `script -q -f -c fleet-daemon`; restarted the same
  way for the orphan check, with the app left running.
- **Disk checks:** `ls`, `git branch -a` and `git worktree list` in the demo repo before and after Archive;
  `GET /api/projects` for the records.

## Result

Passed, after one bug the run found was fixed (below).

| Check | Result | Evidence |
|---|---|---|
| Header counts, two running | `Projects: ● 2 running ● 0 stopped · 0 B` right of the Repo filter, before any stop. Running projects are not measured until a stop or Refresh, hence 0 B. | — |
| Stop from the card | Stop, then Confirm Stop on "add retry to uploads": the card moved to Stopped with the amber Stopped badge, "stopped by you", 5.7 KB and a Details button; the header went to `● 1 running ● 1 stopped`. | `1-…` |
| Crash (tmux session killed by hand) | `tmux -L fbt10 kill-session` on "fix login timeout": the card moved to Stopped with the red Crashed badge and "crashed while waiting at startup prompt"; footer warning "fix login timeout crashed: crashed while waiting at startup prompt"; header `● 0 running ● 2 stopped [1 crashed] · 5.7 KB` without waiting for the 10 s poll. | `1-…` |
| Clicking the crash line | A third project ("tidy docs") was spawned and crashed the same way; clicking its footer line opened that card in the side panel (card outlined) and dismissed the line (sticky warnings on). | — |
| Stopped card's panel | Terminal tab shows the saved last screen (the double's `cwd=… token=… arg=… ready`); it stays after the stream ends. No reply box. Crashed cards' Terminal tab reads "not saved". See Observations for what the stream does afterwards. | `1-…` (screen painted over) |
| Details on a stopped card | Opened Projects with every project listed (no filter) and "add-retry-to-uploads" selected: Stopped badge, folder, goal, repo `demo` from main at `74cfb48`, footprint (demo worktree 107 B, daemon files 5.6 KB, total 5.7 KB), activity, log ("started with demo", "stopped by you"), diff. | — |
| Archive through the dialog | Archive… listed the worktree (107 B), the agent folder under `state_dir` (5.6 KB) and the project folder (0 B, "stays"), "Frees 5.7 KB", "Branches are kept". After Archive: the project folder is gone; `fleet/add-retry-to-uploads` is still in `git branch -a` (no longer checked out); `git worktree list` no longer lists it; the agent folder is gone; the record is `archived` with `removed` (worktree, `CLAUDE.md`, `PROMPT.md`, agent folder, project folder), `freed: 5735`, `kept: null`; footer "add-retry-to-uploads archived, 5.7 KB freed"; the card left the board. | `3-…` (archived row) |
| Refresh re-fetch | Crashed projects showed "not measured yet". Refresh on "tidy docs": 1.2 s later still "not measured yet", 4.7 s later the footprint (5.7 KB, "measured 3s ago"), before the next 10 s poll was due. | — |
| Orphan after a daemon restart | A folder made by hand under the projects root (`left-behind`, 300 KB) and the daemon restarted: an Orphan row (amber badge, 300.0 KB) with "A folder under the projects root with no project record"; totals `1 orphan`, `311.4 KB held / reclaimable`; header `[1 orphan]` badge. Clicking it in the header opened Projects filtered to orphans ("Showing orphans", one row). | `2-…`, `2b-…`, `3-…` |
| Show archived | The archived project is listed dimmed with an Archived badge only with the toggle on. | `3-…` |

## Bug found by the run, fixed

1. **Header badges took the header's full height.** The "1 crashed" badge in the Kanban header painted its red fill
   from the top of the header row to below it (39 px tall around 13 px of text): a `Frame` inside the header's
   full-height, vertically centred row grows to the row. The kittest tests found the labels and clicked them but did
   not look at the frame. Fix: each count is drawn in a row one badge tall (`COUNT_HEIGHT`, the cards' badge row).
   Test: `header_badges_are_as_tall_as_their_text_not_the_header` (`tests/app.rs`, fails before the fix with the badge
   at y 8–47 around text at y 21–34). Re-checked in the real window (`2b-…`). Folded into
   `575c292 feat(gui): show project counts on the board`.

## Observations (not fixed)

- **A stopped agent's terminal stream reconnects forever.** For a stopped agent the daemon sends the saved screen
  once and ends the SSE response without a `closed` event (`crates/fleet-daemon/src/http.rs`, the
  `Status::Stopped` branch of the pane stream); the client only stops on `closed`
  (`crates/fleet-client/src/stream.rs`), so it reconnects with backoff and fetches the same screen again for as long
  as the panel is open. The screen on show does not change, but the "Session ended" note never appears. Sending
  `closed` after the saved screen would end it.
- **Crashed projects are not measured.** A crash does not trigger the measurer (only startup, Stop and Refresh do), so
  a crashed card has no size and the header's disk total leaves it out until a Refresh or a daemon restart. The spec's
  measurement points do not name a crash; whether they should is a question for Antonio.
- **Add repo is offered in a stopped card's panel.** The panel hides the reply box for a stopped agent but still
  shows "Add repo" next to its repos. Not tried; likely should be hidden like the reply box.
- **No ⚠ before the orphan count.** The bundled fonts carry no ⚠ glyph, so the header uses the Projects view's amber
  "Orphan" look: a `N orphan` badge.
- **Stopped dot colours differ.** The header's stopped dot is the neutral grey of the mockup; the board's Stopped
  column dot is the schema's purple.
- The panel's terminal in `1-…` and the folder paths in `3-…` are painted over: the test double prints the agent
  token and local paths.

## Not exercised

- A real `claude`: "running" means a live tmux session at the startup prompt.
- An Exited cause (`/exit` in the agent's terminal), archiving an orphan, End session on an orphan session, an
  archive that keeps something, and a dirty worktree in the plan: covered by the daemon and kittest tests, not driven
  in the window.
- The light theme, `./build.sh` (release build with the web UI), and any machine other than this one.

## Cleanup

Stopped by recorded pid after checking each process's `HOME`: the app, the daemon (its `script` wrapper exited with
it), Xvfb `:97`. The `fbt10` tmux server had no sessions left (`no server running`). Port 7963 free afterwards. The
scratch home stays under `.superpowers/` (git-ignored).
