# egui client v0.1 acceptance (2026-10-23)

Run on the Rocky 8 workstation with the real `claude` 2.1.309 (Antonio's login), on throwaway repos, with the zig-built binaries from `dist/` (glibc 2.28) and the window on a virtual X display (`Xvfb :99`, software GL). Screenshots are in `egui-acceptance/`. The checklist is `docs/acceptance-gui.md`. No `xdotool` exists here, so clicks inside the real window were not driven; click-level behaviour is covered by the UI tests (67 in `fleet-gui`), and the flows below that need a real agent were driven through the daemon API while the window showed the result.

## Result

Passed, after four bugs the run found were fixed (below).

| Check | Result |
|---|---|
| Installer in a clean home with real tmux 3.8 and claude | Installed both binaries, config template, launcher (`bash -lc "exec …"`), icon; printed the trust step; a second run upgraded in place and kept the config. `fleet-gui --version` and `fleet-daemon --version` print 0.1.0. |
| Start the app with no daemon running | `daemon.pid` holds a live `fleet-daemon`; `daemon.log` shows it listening; the window showed the board, the live indicator, five columns, filter, spawn form (`1-…png`). |
| Two real agents in two repos | Both started with no trust dialog (project folder under the trusted root). Core reached Needs you with a `permission` badge and "wants to run Bash"; portal finished into Review (`2-…png`). |
| Answer, then Review with changes | Answer `1` through the API: Working, then Review with a "Has changes" badge and the file inside the repo's worktree (`3-…png`). |
| Pause and Resume mid-tool | Card showed the Paused badge with Resume and Kill and no Pause (`6-…png`); Resume let the agent continue. |
| Kill | Every agent killed: no cards, project folders, branches or tmux sessions left. |
| Daemon down under a running window | Window stayed responsive, indicator "reconnecting…", cards kept (`4-…png`). |
| Daemon back | The window reattached by itself ("live"); the blocked agent showed "status unverified since daemon restart" and finished when answered through the restarted daemon (`5-…png`). |
| Kill only the app process | Daemon kept running with the same pid and still answered; a new window picked up the existing cards immediately. |
| SIGTERM with the window connected | Daemon exited after 2.0 s, pidfile removed, the two agents' tmux sessions kept running; a restart adopted both. |
| Elapsed-time labels on a quiet board | Advanced between two captures 35 s apart (41 s to 1 m). |
| Close dialog, "Stop the daemon too", remember, Ask me again | Covered by the UI tests, not the real window. |

## Bugs found by the run, fixed

1. **Daemon never exited on SIGTERM while a client was connected.** Graceful shutdown waits for open connections, and an event stream never ends, so "Stop the daemon too" would hang and then fail after 5 s. Now the daemon stops accepting, gives requests 2 s, and exits regardless. Test: `sigterm_stops_the_daemon_promptly_even_with_an_open_event_stream`.
2. **`fleet-daemon --version` was missing** although the design lists it. Test: `the_binary_prints_its_version_without_starting`.
3. **Elapsed-time labels froze on an idle board.** The window now redraws every 15 s. Verified in the live window; no automated test (the harness cannot observe a delayed repaint).
4. **Installer test measured the machine's login-profile dotfiles** as installer output; it now measures only what the installer creates.

## Not exercised

- Real clicks in the window (no `xdotool`); the close dialog in the real window.
- The Diff tab and terminal preview in the real window (covered by tests; the pane and diff were not opened by hand).
- Any machine other than this one: Nobara, Wayland, a different GPU.
