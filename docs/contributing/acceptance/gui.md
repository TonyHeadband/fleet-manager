# egui client v0.1 acceptance checklist

Run on the workstation against the real `claude`. Everything the old web checklist (`docs/acceptance.md`) checks applies to the daemon; this one checks the app and the install. Screenshots come from a window on a virtual X display (`Xvfb`); click-level behaviour is covered by the UI tests (`cargo test -p fleet-gui`).

## Install

- [ ] `dist/packaging/install.sh` in a clean home reports `tmux` 3.2 or newer and `claude`, installs `fleet-daemon` and `fleet-gui` to `~/.local/bin`, adds the menu entry and icon, and prints the one-time trust step naming `~/fleet-projects`. It writes no config. Running it again upgrades in place and keeps an existing config.
- [ ] With tmux older than 3.2 on the login PATH it stops before installing anything and says how to fix it.

## First run

- [ ] With no `~/.config/fleet-manager/config.toml`, `fleet-gui` opens on "Welcome to fleet-manager" and does not start a daemon yet. The field holds `~/fleet-projects`.
- [ ] Continue creates the folder, writes `config.toml` with `projects_root = "<path>"` plus the commented template, starts the daemon and lands on an empty board.
- [ ] Trust the projects folder once (`mkdir -p ~/fleet-projects && cd ~/fleet-projects && claude`; pick "Yes, I trust this folder", which is the second choice; `/exit`). Do not answer `1` to the dialog: it is "No, exit". The first-run screen's Copy button copies the command.
- [ ] Skipping the trust step shows the first card as Needs you "waiting at startup prompt"; in the agent's terminal (`tmux -L fleet-manager attach -t <session>`) choose "Yes, I trust this folder" with Down, then Enter (the second choice; Enter alone or `1` is "No, exit" and ends the agent, so do not answer from the reply box).

## The app

- [ ] Starting `fleet-gui` with no daemon running starts one (detached): `<state_dir>/daemon.pid` holds a live `fleet-daemon` pid, `daemon.log` shows it listening, the window shows the board with the live indicator.
- [ ] With a daemon that cannot start (for example tmux 2.7 first on PATH) the window shows the daemon's own reason and the log path, with Retry.
- [ ] The five columns (Stopped, Queued, Working, Needs you, Review), repo filter and the New project dialog behave as in the spec: spawn a project; an agent goes Working then Needs you with a `permission` badge; answering from its card returns it to Working and then Review with a "Has changes" badge; pausing mid-tool and resuming; Stop with the second confirming click (Stop, then Confirm Stop) moves the card to Stopped with a "Stopped" badge and "stopped by you", ends the session and keeps the worktrees, the branches and the project folder; the footer reads "<task> stopped".
- [ ] The Stopped column comes first, on a darker background than the others. Kill a running agent's tmux session by hand (`tmux -L fleet-manager kill-session -t <session>`): its card moves to Stopped with a red "Crashed" badge and "crashed while <activity>", its size on disk, and no Stop; the footer warns "<task> crashed: crashed while <activity>". An agent that exits on its own (`/exit` in its session) shows "Exited" and the footer reads "<task> exited".
- [ ] Every card has the same height in every column, whatever its content: a long task is cut after two lines with "…", a long activity after one, badges that do not fit collapse into "+N"; hovering any of them shows the whole text.
- [ ] Clicking a Stopped card opens the side panel with no reply box: Terminal shows the screen saved when it stopped, Diff works as usual.
- [ ] Clicking a card opens the preview: the terminal text follows the agent and is not editable; the Diff tab lists each repo's changes and Refresh reloads it.
- [ ] Restarting the daemon while an agent waits: the card returns with "status unverified since daemon restart" and the next event replaces it.
- [ ] Killing the app process leaves the daemon and the agents running.

## Projects and repos

- [ ] New project with a goal and no repos: the card appears, the agent works in an empty `<projects_root>/<slug>/` that holds only `CLAUDE.md`.
- [ ] New project with a path repo and a URL repo (chips in the dialog): the folder holds one worktree per repo on `fleet/<slug>`, the URL repo is cloned once into `<state_dir>/repos/<name>`, `CLAUDE.md` lists both, and the side panel's Repos row shows both chips.
- [ ] The next New project offers both repos as picks (the remembered list).
- [ ] Add repo in the side panel with a path, a URL and a remembered name: each adds a worktree, rewrites `CLAUDE.md`, and the agent's session gets a "repo X is now available at ..." line. Adding the same repo again changes nothing. A bad source shows an error that lists the accepted forms.
- [ ] An agent calls `request_repo` (no repo attached): the card moves to Needs you with the "Repo access" badge and "wants repo ...", with no permission prompt in the session. Open the panel: it shows the Repo request box in place of the reply box (a freshly started app does; see `findings/projects-first.md` for the live-update gap).
- [ ] Grant with the target edited (for example a path changed to a URL): the held tool call returns the new worktree path and the agent continues inside it.
- [ ] Refuse with a reason: the agent receives `refused: <reason>` and continues.
- [ ] An agent runs `git init` in a subfolder: within a few seconds it shows as a repo on the card and in Diff (everything is an addition) and is remembered.
- [ ] Stop on that project moves the card to Stopped and keeps the folder, the `git init`ed repo, the worktrees and the branches.
- [ ] Browse… sits next to each of the four path fields (New project "Path or git URL", side panel Add repo, repo request "Grant access to", and the first-run screen's projects folder, which picks folders only); each opens the picker, Select fills the field, Cancel and Escape leave it unchanged.
- [ ] In repo mode the list shows checkouts with their branch and a "N uncommitted files won't be included" note on dirty ones; hovering a checkout on a branch shows the merge-back note with `fleet/<project>`.
- [ ] In repo mode, folders under the projects root are greyed and cannot be picked.
- [ ] Start from a checkout on branch `feat` with commits: the worktree starts at that HEAD, the chip tooltip reads "fleet/<slug>, started from feat at <sha>" followed by the merge-back note, and merging `fleet/<slug>` into `feat` fast-forwards.

## Projects view

- [ ] The drawer lists Projects between Kanban and Settings. The view opens with a totals row (running, stopped, crashed and orphans counted apart; disk held and reclaimable; the clones cache; agents started; tokens and cost "not tracked yet"), a narrow list on the left and the selected project's passport on the right. The list refreshes by itself within 10 s of a change (Stop on the board, a crash).
- [ ] List order: running projects (in their board status colour), stopped ones with crashed first (Stopped amber, Exited blue, Crashed red), orphans (amber "Orphan"), then archived ones, dimmed and only with "Show archived" on; newest first within each.
- [ ] A running project's passport: name, badge, started time, folder, goal, repos with branch and start commit, Footprint with each worktree's `of which target/` line and "measured <ago>"; Refresh re-measures (the time updates within a few seconds). Activity shows agents used, time running and per status, and tokens · cost "not tracked yet". Archive… is greyed with "stop it first"; Stop, then Confirm Stop, stops it, and its row turns Stopped.
- [ ] A stopped project's passport adds Diff (Refresh reloads it) and Last screen (the screen saved at Stop; "not saved" after an exit or a crash). Open on board switches to Kanban with its card's side panel open.
- [ ] Archive… on a stopped project with an uncommitted file in a worktree: the dialog shows "Measuring…" with Archive greyed, then each worktree and the daemon's files with sizes, "1 uncommitted file will be lost" on the dirty worktree, the project folder as "stays", and "Frees <size>". Cancel has the focus; Enter or Escape cancels and nothing changes on disk. Archive removes the worktrees (the `fleet/<slug>` branches stay), the footer reads "<project> archived, <size> freed" (or "…, kept <path>" when a file of yours was left in the folder), and the project moves to the archived rows with a read-only passport and no Open on board.
- [ ] An orphan: make a folder under the projects root by hand and restart the daemon. It shows as an Orphan row with its size; Archive… shows the same dialog and, once confirmed, removes it and leaves an archived record. A tmux session with the daemon's prefix and no record (`tmux -L fleet-manager new -d -s fleet-zz`) shows End session…, which asks "An agent may be working in this session. End it?" with Cancel focused; End session kills it.

## App shell (drawer, Settings, footer, log)

- [ ] Header and footer lines are continuous in both themes: the drawer header and the content header are the same height and share one unbroken bottom border from the window's left edge to its right; the drawer footer and the content footer share one unbroken top border. Check on Kanban with the detail panel open and on Settings, in Dark and in Light (a full-width pixel row of the border colour in a screenshot).
- [ ] Nav switching with an agent selected: select a card on Kanban (the detail panel opens), click Settings (no detail panel), click Kanban (the detail panel is back on the same agent). The drawer footer reads `live` and `N agents · M need you` (the second part only when someone waits).
- [ ] Footer cycling with errors, "Keep errors and warnings in the footer until dismissed" on: two errors (for example Add repo with two missing paths) alternate, 8 s each, with a counter that reads `1/2` then `2/2`; neither leaves by itself; clicking the line dismisses the one shown, and the other then stays alone with no counter.
- [ ] The same with the setting off: an error shows once for 8 s, then stays dimmed until the next message.
- [ ] History in the footer lists the last messages newest first with time and a coloured dot (including successes such as "Settings saved"); `Open log →` switches to Settings → Log.
- [ ] Projects folder Save with a running agent: General → Projects folder, type a new folder, Save. `config.toml` gets the new `projects_root` with its comments and other keys kept; the daemon restarts (new pid, footer "Daemon restarted", drawer back to `live`); the agent's card and tmux session survive ("status unverified since daemon restart"); the next New project lands in the new folder. With an agent still Queued, Save is disabled and its tooltip says why.
- [ ] Log tab with a daemon started from a terminal (`fleet-daemon` by hand, before the app): each log line appears once on the terminal and once in `daemon.log`, not twice. Settings → Log: Both merges app and daemon lines by time; App shows only `fleet_gui`/third-party lines; Daemon only `fleet_daemon` lines; unparsed lines (`fleet-manager listening on …` from a daemon the app started) show as-is with no dot.
- [ ] Killing only the app process after a Save leaves the restarted daemon and the agents running; a new window picks them up.

## Closing

- [ ] Closing with agents Working or waiting shows the dialog with the real counts; "Keep the daemon running" is focused; Escape and Cancel keep the window open and change nothing.
- [ ] "Keep" closes the window only: the daemon and agents survive. "Stop the daemon too" ends the daemon (pidfile gone) and the agents keep running in tmux.
- [ ] "Remember my choice" is saved to `~/.config/fleet-manager/gui.toml` and skips the dialog next time; Settings → General → Closing the window shows the saved choice, and Ask brings the dialog back.
- [ ] With Closing the window on Ask, closing always shows the dialog, even with nothing Queued, Working or waiting and a daemon this app started.

## Known limits in v0.1

- Linux (Rocky 8 and Nobara/Fedora). No Windows or macOS build runs; the code compiles for them as a guard (`FLEET_CHECK_WINDOWS=1`, `FLEET_CHECK_MACOS=1`).
- Characters the bundled fonts lack show as boxes in the terminal preview.
- No notifications, history or review inbox (#50, #51, #52).
