# Projects first: real run on the real `claude`

Run 2026-10-24 (Task 12), Claude Code 2.1.310, release builds of `fleet-gui` and `fleet-daemon` from the working tree, Rocky 8.

## Summary

| Check | Result |
|---|---|
| (a) MCP client reaches `/internal/mcp/{id}`; Host/Origin guard lets it through | PASS |
| (b) `permissions.allow` in the generated `--settings` stops the prompt for `mcp__fleet__request_repo` | PASS |
| (c) Claude tolerates the 405 on GET | PASS, with a caveat (below) |
| (d) `request_repo` shows Needs you + "Repo access"; Grant with an edited target returns and the agent continues in the new worktree | PASS after the fix wave (re-run below, app left running). The first run failed for the live GUI panel: the Repo request box did not appear until the app was restarted |
| (e) A refusal returns its reason | PASS |
| (f) Kill keeps an agent-initialised repo and reports it | PASS |
| (g) Trust dialog on a fresh project folder | Findings below. Answering `1` kills the agent; the guidance now names the second option |

Other things seen are under "Concerns found". Nothing in the daemon or the GUI was changed.

## Isolation

- `HOME` was a temp directory. Claude finds its login in `~/.claude/.credentials.json` (a file, not the keychain), with the account in `~/.claude.json`. I copied only `.credentials.json` (mode 0600) and a minimal `.claude.json` holding `oauthAccount`, `userID` and the onboarding flags. The access token had about 7 h left, so the copy never refreshed and the owner's refresh token was not rotated. The `projects` history and the rest of `~/.claude` were not copied.
- Projects root `<tmp>/proj-root`, private tmux socket (`FLEET_TMUX_SOCKET=fb12`, `TMUX_TMPDIR` under the temp dir), `listen = "127.0.0.1:7993"`, Xvfb on `:97`. The owner's daemon was live on `127.0.0.1:7878` the whole time and was not touched. `~/.config/fleet-board` and `~/fleet-projects` were never read or written.
- Only my own processes were stopped (GUI, daemon, the private tmux server, Xvfb, by recorded pid or socket). The temp directory, with the credential copy, was removed afterwards.
- No config existed at the start. The first-run screen was captured and its Continue wrote the config. Because the default port 7878 was taken by the owner's daemon, the GUI for that step ran with no `fleet-daemon` on its PATH, so it stopped on "could not start its daemon" after writing the file. I then fixed the root's letter case (my typing script had lowercased a capital), added `listen` and `claude_command` to the written file, and started the GUI with the daemon beside it.
- I set `claude_command = ["claude", "--permission-mode", "default"]`, as the config template recommends. The temp `HOME` still picked up this machine's managed Claude settings, which make auto mode the default; the default `claude_command` of plain `claude` would therefore run agents in auto mode here and the Needs-you permission cards would not fill. Not tested without the flag.

## (a)(b)(c) MCP on the real CLI

- A project with no repos was started from the API. Its prompt told the agent to call `request_repo`. The session showed "Calling fleet" and the daemon recorded the request, so the client got through `/internal/mcp/{id}` and the Host/Origin guard did not refuse it (a).
- The session never showed a "Do you want to proceed?" prompt for the tool; the footer read "manual mode on". The generated `settings.json` carries the `permissions.allow` entries, so they work (b).
- `curl -X GET /internal/mcp/<id>` returns 405. The agent's `initialize`, `tools/list` and `tools/call` all worked, so the 405 does not stop the client (c). The daemon does not log requests, so I cannot show that Claude Code sent the GET this time; the spike saw it send one.
- The call was held about two minutes (03:35:43 to 03:38) with no "moved to background" notice and no timeout, so the three env vars from the spike are being applied.

## (d) Grant with an edited target

- Request: `repo=<tmp>/repos/alpha`, why "read its README". The card moved to Needs you with the "Repo access" badge and "wants repo <path>" (`board-repo-access.png`).
- **The side panel did not show the Repo request box while the app stayed running** (`panel-live-no-request-box.png`: no box, no reply box, no Repos row). After closing and reopening `fleet-gui` the same panel showed the box, the Repos row and the "Add repo" control (`panel-request-box.png`). Cause, from reading the code: `EventKind::StatusChanged` (`crates/fleet-api/src/event.rs:12`) carries `blocked_reason` and `activity` but not `repo_request`, so a client that is already connected never learns of the request; only a fresh snapshot has it. The board and the badge update live; the box does not. This is for the controller to route.
- In the restarted GUI I edited "Grant access to" from the path to `file://<tmp>/repos/urlrepo.git` and clicked Grant. The held call returned, a URL clone appeared at `<state_dir>/repos/urlrepo`, a worktree `<project>/urlrepo` on `fleet/grant-test` was made, `CLAUDE.md` was rewritten, and the agent ran `ls` in the new worktree and replied "DONE README.md". The card went to Review.

## (e) Refusal

Refused through the API with a reason. The agent replied with exactly that reason ("REFUSED: License review is not needed for this project, work from urlrepo instead.") and continued.

## (f) Kill and an owned repo

- The agent was asked to make `scratch/`, `git init` it and write a file. After the Bash permission was answered, the daemon registered `scratch` within the next tick as a workspace with `owned: true` and base `4b825dc`, and remembered it (it shows in `GET /api/repos` as a path repo).
- Kill returned `{"kept":"kept <root>/owned-test: it contains scratch"}`. The folder had lost `CLAUDE.md` and kept `scratch/` with its `.git` and `hello.txt`. Kill on the other two projects returned `kept: null` and removed their folders; the worktree was gone from the base's list and the branch `fleet/grant-test` survived in the URL clone.

## (g) The trust dialog

- Root not trusted, fresh project folder: the session stops on "Quick safety check: Is this a project you created or one you trust?" with **"No, exit" pre-selected** and "Yes, I trust this folder" second. The status stayed Queued for a few seconds and then became Needs you, "waiting at startup prompt", blocked reason `startup`, as designed.
- **Answering `1` (what the spec says to type) selects "No, exit"**: Claude quit, the tmux session ended and the card became Dead "session lost". The right keys are Down then Enter. The dialog shows no numbers. The first-run screen text and the acceptance doc should say "choose Yes, I trust this folder", which is what the doc now says. I did not test typing `2`.
- Trusting the root first: `cd <root> && claude`, Down, Enter, `/exit` wrote `hasTrustDialogAccepted: true` for the root only. A new project folder under it then started with no dialog: the agent went straight to its work. Trust inheritance from the root holds.

## Also checked (briefly)

- Project with a path repo and a URL repo, started from the New project dialog (`new-project-chips.png`): both worktrees on `fleet/<slug>`, `CLAUDE.md` lists both and imports `alpha/CLAUDE.md` (`@alpha/CLAUDE.md`).
- Manual add by path, then again by name: the second returned the same workspace. A bad source returned 400 naming the accepted forms. The agent's session got "fleet-board: repo gamma is now available at ...".
- No config: the GUI shows the first-run screen (`first-run.png`) and no daemon is started before Continue.

## Concerns found (not fixed, for the controller)

1. **Live panel misses the request box** (above): add `repo_request` to the status event or send a snapshot refresh.
2. **Remembered repos are not refreshed in the GUI.** The New project dialog showed no picks while `GET /api/repos` held two entries (`urlrepo`, `scratch`) that were remembered after the app started.
3. **The first-run screen's layout:** the Copy button wraps one letter per line ("C / o / py") and the trust line is cut after "/exit" (`first-run.png`).
4. **The side panel's header path reads "/"** for a project with no repo or after a grant (`panel-request-box.png`); the field is the panel's `path`.
5. **Default `claude_command` is plain `claude`**, which follows the owner's managed default (auto mode on this machine), so permission cards may never appear. The template comment recommends `--permission-mode default`.
6. `scratch`, an agent's own `git init` repo, is remembered and offered as a pick for other projects even though it may have no commits (a worktree of it would fail). Not tried.

## Fix wave: (d) re-run with the app left running

Run 2026-10-24 after the fix wave, Claude Code 2.1.310, release builds from the working tree.

- Isolation as above: temp `HOME` with only `.credentials.json` (0600, about 6.5 h left, so never refreshed) and a minimal `.claude.json` (`oauthAccount`, `userID`, onboarding flags, and `hasTrustDialogAccepted` for the temp projects root only); `FLEET_TMUX_SOCKET` private with `TMUX_TMPDIR` in its own temp dir; a free port (59559); Xvfb on `:93`. The owner's daemon on 7878, `~/.config/fleet-board` and `~/fleet-projects` were not touched. Only the processes started for the run (two GUIs, the daemon the first GUI started, Xvfb) were stopped, by recorded pid; the private tmux server was gone after Kill; the temp dir with the credential copy was removed.
- `fleet-gui` was started first, on an empty board, and stayed open for the whole check. A project with no repo was then started from the API, its prompt asking for `request_repo` on `<tmp>/repos/alpha`.
- **The box appeared live.** Within seconds the card moved to Needs you with "Repo access". Opening it showed the Repo request box ("Wants `<tmp>/repos/alpha`", "read its README"), the Repos row with "Add repo" although the project had no repo and nothing was remembered, and the project folder as the header path (`panel-live-request-box.png`). No restart.
- **Grant with an edited target.** The "Grant access to" field was changed to `file://<tmp>/repos/urlrepo.git` and Grant clicked. Within 3 s, still without a restart, the box was gone, the card was back in Working, the Repos row showed `urlrepo` and the header read `urlrepo / fleet/live-grant` (`panel-live-after-grant.png`). The daemon had cloned the URL to `<state_dir>/repos/urlrepo`, made the worktree `<project>/urlrepo` on `fleet/live-grant` and rewritten `CLAUDE.md`.
- **The agent continued.** The held call returned the grant (nothing was typed into the session), the agent ran `ls` in the new worktree, replied "DONE README.md" and the card went to Review.
- The New project dialog, opened afterwards in the same app, offered `urlrepo` as a pick (remembered after the app started). A second GUI with an empty `HOME` showed the first-run screen with the new trust line on one row and a normal-sized Copy button.
- Kill returned `{"kept":null}`.

| Concern | Status |
|---|---|
| D1 Live panel misses the request box | Fixed: `StatusChanged` carries `repo_request`; seen live above |
| D2 Remembered repos not refreshed | Fixed: fetched again when the New project dialog opens and after a spawn, an add and a decision; seen live above |
| D3 First-run layout (Copy one letter per line, trust line cut) | Fixed: the hint wraps on its own line, the command and a fixed-size Copy sit under it |
| D4 Panel header path reads "/" | Fixed: the project folder while there is no repo, recomputed every frame (updated after the grant above) |
| D5 Trust guidance ("accept", "answer 1") kills the agent | Fixed: the first-run screen, `install.sh`, the spec and `acceptance-gui.md` say to choose "Yes, I trust this folder", the second option (Down, Enter) |

## Captures (`projects-first/`)

| File | Shows |
|---|---|
| `first-run.png` | First-run screen, no config (dark) |
| `board-repo-access.png` | Board with the Repo access card in Needs you |
| `panel-live-no-request-box.png` | Panel on that card in the running app: no request box |
| `panel-request-box.png` | Same card after restarting the app: Repos row and the Repo request box |
| `new-project-chips.png` | New project dialog with a goal and two repo chips |
| `panel-live-request-box.png` | Fix wave: the Repo request box in the app that was already running when the request came |
| `panel-live-after-grant.png` | Fix wave: 3 s after Grant with the edited target, same app: box gone, `urlrepo` in the Repos row and the header |

## Gates

See the report; `cargo test --workspace`, clippy and the Windows check were run at the end.
