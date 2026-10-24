# App shell acceptance (2026-10-24)

Run on the Rocky 8 workstation against the `feat/app-shell` release build (`cargo build --release -p fleet-daemon -p fleet-gui`; `./build.sh` was not used because no `zig` is installed here), with the window on a virtual X display (`Xvfb :99 -screen 0 1400x800x24 +extension GLX`, software GL). Screenshots are in `app-shell/`. The checklist is the "App shell" section of `docs/acceptance-gui.md`. Spec: `specs/2026-10-24-app-shell-design.md`.

## How it was run

- **Isolation:** `HOME` was a scratch folder under the worktree's `.superpowers/acceptance-home/` (git-ignored), with its own `config.toml` (`listen = "127.0.0.1:7961"`, `projects_root = "~/fleet-projects"`, `claude_command` = the test double), its own `gui.toml` and `.gitconfig`; tmux on its own socket (`FLEET_TMUX_SOCKET=fbacc-appshell`). Antonio's config, state, daemon on 7878 and tmux sessions were not touched.
- **No real Claude:** agents ran `crates/fleet-daemon/tests/bin/claude`, so every card sits in Needs you with "Startup prompt" after the startup grace. That is the double, not the feature.
- **Clicks and typing:** a Python `ctypes` script driving `libXtst` (XTEST pointer and key events) plus `XSetInputFocus` on the window, kept under `.superpowers/`. Keys only arrive after the window is focused explicitly (there is no window manager).
- **Daemon from a terminal:** started by hand before the app, on a pseudo-terminal (`script -q -f -c fleet-daemon`), so its stderr was a terminal; the app then found it running.
- **Measurements:** `convert … -crop 1400x1+0+Y … histogram:info:` on full-width pixel rows of each screenshot.

## Result

Passed, after one bug the run found was fixed (below).

| Check | Result | Evidence |
|---|---|---|
| Header and footer lines continuous, dark | Kanban with the detail panel open and Settings → General/Log: pixel row y=39 is 1400/1400 px `#333333` (the border) and row y=772 is 1400/1400 px `#333333`. The rows above and below are the drawer/content fills (`#212121` / `#1C1C1C`) with only the 1 px vertical separators in the border colour. Both headers span y=0–38 and both footers y=773–799. | `1-kanban-dark.png`, `3-…`, `4-…`, `zoom-split-dark.png` |
| Header and footer lines continuous, light | The same rows: y=39 and y=772 are 1400/1400 px `#D9D9D9` on Kanban (detail open), Settings → General and Settings → Log. | `2-kanban-light.png`, `9-settings-general-light.png`, `zoom-split-light.png` |
| Nav switching with an agent selected | Card selected, detail panel open; Settings showed no detail panel; back on Kanban the detail panel showed the same agent. Drawer footer: `live · 1 agent · 1 needs you`, later `2 agents · 2 need you`. | `1-…`, `3-…` |
| Footer cycling with errors, sticky on | Two Add repo errors (`/nonexistent/one`, `/nonexistent/two`) alternated, 8 s each, kept looping; clicking the line dismissed the one on screen, a second click the other. The counter read `1/2` for both before the fix, `1/2` / `2/2` after it. | `11-footer-cycle-after-fix.png` (8 captures 2.5 s apart) |
| Footer with sticky off | Unticked the setting (`gui.toml` got `sticky_alerts = false`, footer "Settings saved"); a new error showed for about 8 s and then stayed dimmed. | `12-footer-not-sticky.png` |
| History and `Open log →` | History listed four messages newest first with UTC time and dot (three errors and "Settings saved"); `Open log →` opened Settings → Log. | `8-footer-history-dark.png` |
| Projects folder Save with a running agent | Typed `~/fleet-projects-2`, Save. `config.toml` got `projects_root = "~/fleet-projects-2"  # the first root` with its comment and the other keys kept; the hand-started daemon (pid 1554921) stopped and a new one (pid 1563878, child of the app) started; footer "Daemon restarted", drawer `reconnecting…` then `live`. The agent kept its card and its tmux session `fleet-eb2ec30d` ("status unverified since daemon restart"). New project "second agent after the move" landed in `~/fleet-projects-2/second-agent-after-the-move`; footer "Agent started". | `3-settings-general-dark.png`, `4-…` |
| Save disabled while an agent is Queued | Right after that spawn, Save was greyed with the tooltip "An agent is still starting; wait until it runs". | `10-save-disabled-queued-dark.png` |
| Log tab, daemon started from a terminal | The terminal showed `… INFO  fleet_daemon daemon started listen=127.0.0.1:7961 version=0.1.0` once, plus the CLI line `fleet-manager listening on …`; `daemon.log` had the log line once and not the CLI line. After the restart (daemon started by the app) `daemon.log` got the new `daemon started` line once and the CLI line as an unparsed line. `sort daemon.log | uniq -d` is empty. Both merged app and daemon lines by time; App showed only `fleet_gui` and winit lines; Daemon only `fleet_daemon` lines; the unparsed line shows as-is with no dot. | `4-…`, `5-settings-log-app-dark.png`, `6-settings-log-daemon-dark.png` |
| Settings → Daemon | Address, "already running", state directory, config file, PID, Restart daemon. | `7-settings-daemon-dark.png` |
| Kill only the app after the Save | The restarted daemon kept running with the same pid; the rebuilt app started against it and showed both agents. | — |

## Bug found by the run, fixed

1. **The footer counter always read `1/n`.** `components/footer.rs` drew `format!("1/{n}")` and ignored the position `Messages::position` returns, so the second of two cycling errors still read `1/2`. The queue's own test covered `position()`, not what the footer draws. Fix: draw `{i}/{n}`. Test: `the_footer_counter_shows_the_message_on_screen` (kittest, `tests/messages.rs`). Re-checked in the real window: `1/2` then `2/2` (`11-…`). Commit `fix(gui): show the footer counter's position`.

## Observations (not fixed)

- **Restart progress is not in the footer.** The spec says Save restarts the daemon "with progress in the footer"; the build shows "Restarting the daemon…" in place of the note under the Projects folder field (and in Settings → Daemon), and the footer only gets "Daemon restarted" at the end. The drawer footer's `reconnecting…` covers the gap. Either accept it in the spec or post an info message when the restart starts.
- **The daemon log is thin.** In this run `daemon.log` held only `daemon started` lines: no line for a spawn, for the failed Add repo requests, or for the SIGTERM shutdown. Daemon → Log therefore shows almost nothing during normal use. Spawn and shutdown at info would make the Daemon source useful.
- **Third-party warnings at start.** Every app start logs two winit warnings ("error setting XSETTINGS…", "XRandR reported that the display's 0mm in size…"). They come from Xvfb, stay in the log (capped at warn as designed) and do not reach the footer.
- The terminal preview and the panel's folder header in screenshots `1-…`, `2-…` and `8-…` are painted over: the test double prints the agent token and local paths there.

## Not exercised

- `./build.sh` (no zig on this machine); the run used a plain cargo release build.
- A real `claude`: agents were the test double, so "running agent" means a live tmux session at the startup prompt, not one mid-tool.
- Save with a pending repo request (the confirm), Save while a spawn/add-repo/grant call is in flight, Save with no pidfile, Restart daemon on Settings → Daemon, Browse… next to the projects folder, the System theme, and the search box and level toggles in the Log tab: covered by the UI tests (`cargo test -p fleet-gui`: 258 passed), not driven in the real window.
- Any machine other than this one: Nobara, Wayland, a different GPU or font rendering.

## Cleanup

Stopped by recorded pid after checking each process's `HOME`: the app, then the daemon (pidfile removed by the daemon); `tmux -L fbacc-appshell kill-server`; Xvfb `:99`. Port 7961 free afterwards. The scratch home stays under `.superpowers/` (git-ignored).
