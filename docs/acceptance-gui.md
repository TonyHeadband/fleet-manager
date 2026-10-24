# egui client v0.1 acceptance checklist

Run on the workstation against the real `claude`. Everything the old web checklist (`docs/acceptance.md`) checks applies to the daemon; this one checks the app and the install. Screenshots come from a window on a virtual X display (`Xvfb`); click-level behaviour is covered by the UI tests (`cargo test -p fleet-gui`).

## Install

- [ ] `dist/packaging/install.sh` in a clean home reports `tmux` 3.2 or newer and `claude`, installs `fleet-daemon` and `fleet-gui` to `~/.local/bin`, writes `~/.config/fleet-board/config.toml` from the template, adds the menu entry and icon, and prints the one-time trust step. Running it again upgrades in place and keeps an existing config.
- [ ] With tmux older than 3.2 on the login PATH it stops before installing anything and says how to fix it.
- [ ] Trust the worktrees folder once (`cd ~/.local/share/fleet-board/worktrees && claude`, accept, `/exit`).

## The app

- [ ] Starting `fleet-gui` with no daemon running starts one (detached): `<state_dir>/daemon.pid` holds a live `fleet-daemon` pid, `daemon.log` shows it listening, the window shows the board with the live indicator.
- [ ] With a daemon that cannot start (for example tmux 2.7 first on PATH) the window shows the daemon's own reason and the log path, with Retry.
- [ ] The five columns, repo filter, spawn form and their behaviour match the web board: spawn in two repos; an agent goes Working then Needs you with a `permission` badge; answering from its card returns it to Working and then Review with a "Has changes" badge; pausing mid-tool and resuming; kill with the second confirming click removes the card, session, project folder, worktree and branch.
- [ ] Clicking a card opens the preview: the terminal text follows the agent and is not editable; the Diff tab lists each repo's changes and Refresh reloads it.
- [ ] Restarting the daemon while an agent waits: the card returns with "status unverified since daemon restart" and the next event replaces it.
- [ ] Killing the app process leaves the daemon and the agents running.

## Closing

- [ ] Closing with agents Working or waiting shows the dialog with the real counts; "Keep the daemon running" is focused; Escape and Cancel keep the window open and change nothing.
- [ ] "Keep" closes the window only: the daemon and agents survive. "Stop the daemon too" ends the daemon (pidfile gone) and the agents keep running in tmux.
- [ ] "Remember my choice" is saved to `~/.config/fleet-board/gui.toml` and skips the dialog next time; the top bar offers "Ask me again".
- [ ] With nothing Queued, Working or waiting, and a daemon this app started, closing stops it without asking.

## Known limits in v0.1

- Linux (Rocky 8 and Nobara/Fedora). No Windows or macOS build runs; the code compiles for them as a guard (`FLEET_CHECK_WINDOWS=1`, `FLEET_CHECK_MACOS=1`).
- Characters the bundled fonts lack show as boxes in the terminal preview.
- No notifications, history or review inbox (`TODO.md`).
