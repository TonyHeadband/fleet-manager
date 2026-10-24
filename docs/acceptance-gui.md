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
- [ ] The five columns, repo filter and the New project dialog behave as in the spec: spawn a project; an agent goes Working then Needs you with a `permission` badge; answering from its card returns it to Working and then Review with a "Has changes" badge; pausing mid-tool and resuming; kill with the second confirming click removes the card, session, worktrees and (when nothing else is in it) the project folder; the branches stay.
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
- [ ] Kill on that project removes the card but keeps the folder; the problem line reads "kept <path>: it contains <repo>". The same Kill on a project with only worktrees removes the folder.
- [ ] Browse… sits next to each of the four path fields (New project "Path or git URL", side panel Add repo, repo request "Grant access to", and the first-run screen's projects folder, which picks folders only); each opens the picker, Select fills the field, Cancel and Escape leave it unchanged.
- [ ] In repo mode the list shows checkouts with their branch and a "N uncommitted files won't be included" note on dirty ones; hovering a checkout on a branch shows the merge-back note with `fleet/<project>`.
- [ ] In repo mode, folders under the projects root are greyed and cannot be picked.
- [ ] Start from a checkout on branch `feat` with commits: the worktree starts at that HEAD, the chip tooltip reads "fleet/<slug>, started from feat at <sha>" followed by the merge-back note, and merging `fleet/<slug>` into `feat` fast-forwards.

## Closing

- [ ] Closing with agents Working or waiting shows the dialog with the real counts; "Keep the daemon running" is focused; Escape and Cancel keep the window open and change nothing.
- [ ] "Keep" closes the window only: the daemon and agents survive. "Stop the daemon too" ends the daemon (pidfile gone) and the agents keep running in tmux.
- [ ] "Remember my choice" is saved to `~/.config/fleet-manager/gui.toml` and skips the dialog next time; the top bar offers "Ask me again".
- [ ] With nothing Queued, Working or waiting, and a daemon this app started, closing stops it without asking.

## Known limits in v0.1

- Linux (Rocky 8 and Nobara/Fedora). No Windows or macOS build runs; the code compiles for them as a guard (`FLEET_CHECK_WINDOWS=1`, `FLEET_CHECK_MACOS=1`).
- Characters the bundled fonts lack show as boxes in the terminal preview.
- No notifications, history or review inbox (`TODO.md`).
