# Personas acceptance (2026-10-25)

Run on the Rocky 8 workstation against the uncommitted `feat/personas` worktree (Tasks 1–11 plus the Task 12 docs),
with the release binaries `./build.sh` had just put in `dist/` (zig, glibc 2.28) copied out to a scratch folder, and
the window on a virtual X display (`Xvfb :95 -screen 0 1400x800x24 +extension GLX`, software GL). Screenshots are in
`personas/`. Spec: `specs/2026-10-25-personas-design.md`.

This run uses the test doubles for `claude` and `gh`, never the real CLIs: it must not touch Antonio's own
github.com, GHE or Claude accounts. The real-CLI check is the checklist at the end, for Antonio to run himself.

## How it was run

- **Isolation:** `HOME` was a scratch folder under the worktree's `.superpowers/t12-home/` (git-ignored) with its own
  `config.toml`: `listen = "127.0.0.1:7965"`, `projects_root` and `personas_dir` inside the scratch home,
  `claude_command` = `crates/fleet-daemon/tests/bin/claude`, `gh_command` = `crates/fleet-daemon/tests/bin/gh`.
  tmux on its own socket (`FLEET_TMUX_SOCKET=fbt12`). Every process ran under `env -i` with only `HOME`, `PATH`
  (scratch bin with `tmux`, `/usr/bin`, `/bin`), `DISPLAY=:95` and the socket, so no real `~/.claude`,
  `~/.config/gh`, `GH_TOKEN`, `SSH_ASKPASS` or `XDG_*` variable could reach it. Antonio's config, state, daemon on
  7878 and tmux sessions were not touched.
- **Planted ambient credentials:** the daemon alone was started with fake ambient values on top
  (`GH_TOKEN=ambient-fake-token`, `ANTHROPIC_API_KEY=ambient-fake-key`, `SSH_AUTH_SOCK=/nonexistent/agent.sock`,
  `SSH_ASKPASS=/bin/false`, `GIT_CONFIG_PARAMETERS='credential.helper'='store'`), to prove it strips them from the
  login checks and from agents rather than relying on a clean environment.
- **First run:** a second, empty scratch home (`.superpowers/t12-fresh/`, no config) for the wizard's step 1 only.
  Next was not clicked there: with no config the app would look for a daemon on the default port 7878.
- **Logins:** made with the doubles' files: `<persona>/claude/.fake-login` makes `claude auth status` report
  logged in, `<persona>/gh/hosts.yml` makes `gh auth status` do so. Removing `hosts.yml` breaks a login.
- **Hosts:** `ghe.example.invalid` and `github.example.invalid`, so nothing could reach a real server. Two one-commit
  repos under the scratch home, `repos/webapp` (origin `https://ghe.example.invalid/acme/webapp.git`) and
  `repos/blog` (origin `https://github.example.invalid/tony/blog.git`), added as path repos.
- **State through the API:** personas, agent config and projects were made with `curl` against port 7965
  (`POST /api/personas`, `POST /api/personas/{id}/check`, `PUT /api/agent-config`, `POST /api/agents`).
- **Clicks:** a Python `ctypes` script driving `libXtst` (XTEST pointer events), under `.superpowers/t12-tools/`.
  Keyboard events sent the same way did not reach the egui text fields (no window manager, so likely no focus),
  so nothing was typed into the window.
- **Agent environment:** read from the fake `claude`'s first lines (`tmux -L fbt12 capture-pane`) and from
  `/proc/<pid>/environ` of the agent process; git was then run in the agent's worktree with that same environment.

## Result

Passed. No product bug found that breaks the feature; three small UI observations and one doc gap below.

| Check | Result | Evidence |
|---|---|---|
| First run, no config | Step 1 of 3 "Where fleet-manager lives": Projects folder `~/fleet-projects` and Personas folder `~/.local/share/fleet-manager/personas`, each with Browse…, and the note that fleet-manager never reads the tokens. Nothing written but `gui.log`. | `1-…` |
| Wizard resumes a draft | A draft persona (`Rodeo`, `ghe.example.invalid`) made through the API with no ready persona: the app opened straight at Step 3 of 3 · Log in. | `2-…` |
| Step 3, Claude ✓ / GitHub ✗ | Claude line green with `tester@example.com`; GitHub · ghe.example.invalid amber with gh's own "You are not logged into any GitHub hosts…"; **Finish disabled**. The daemon's planted `GH_TOKEN` did not count as a login. | `2-…` |
| Finishing | After `hosts.yml` was written, Check now turned GitHub green (`tester`), Finish enabled, Cancel gone; Finish closed the wizard and the drawer switcher appeared with Rodeo active (saved as `active_persona` in `gui.toml`). | — |
| Persona folders | `<personas_dir>/<id>`, `…/claude`, `…/gh` all mode 700. | — |
| Agent environment | In `fix login timeout` (Rodeo): `CLAUDE_CONFIG_DIR` and `GH_CONFIG_DIR` the persona's, `GH_HOST=ghe.example.invalid`, `GIT_AUTHOR_*`/`GIT_COMMITTER_*` = Antonio Frontino / antonio@work.example.invalid, `GIT_ASKPASS=` (empty), `GIT_ALLOW_PROTOCOL=https:file`, `GIT_TERMINAL_PROMPT=0`; `GH_TOKEN`, `ANTHROPIC_API_KEY`, `GIT_CONFIG_PARAMETERS`, `SSH_ASKPASS`, `SSH_AUTH_SOCK` all **unset** although the daemon had them. `write the about page` (Personal) had Personal's folders, `github.example.invalid` and tony@home.example.invalid. | tmux panes |
| git in the worktree | `git config --get-all credential.helper` = an empty entry, then `!…/tests/bin/gh auth git-credential`. A commit was authored and committed as the persona. `ext::` transport refused ("transport 'ext' not allowed"). `git push --dry-run` went to `https://ghe.example.invalid/…` and failed on DNS without a prompt. The user's checkout's `.git/config` and the scratch `~/.gitconfig` had no credential or persona lines. | — |
| Instructions and settings | The project's `CLAUDE.md` starts with "# Instructions for every agent" and the saved text; the agent's `settings.json` has fleet's MCP allows plus the saved allow/deny rules and `env.RUST_BACKTRACE=1`. A `PUT` with env `GH_HOST` → 400 "GH_HOST is set by the daemon and cannot be overridden here". | `4-…`, `5-…` |
| Strict host rule | SSH URL → "…is an SSH URL; personas only use HTTPS"; a GHE URL under Personal → "…is on ghe.example.invalid, but persona personal is for github.example.invalid…"; Rodeo's path repo under another persona → "…already belongs to persona rodeo". All 400, nothing created. | — |
| Spawn guards | Draft persona → "Draft has not finished logging in to Claude and GitHub"; no persona → "pick a persona for this project". | — |
| Broken login | `Side gig` made ready, one project spawned, then `hosts.yml` removed and checked: the next spawn failed fast with "Log in to Side gig's GitHub again (You are not logged into …)"; the running project carried on (reached Needs you). Management shows ⚠ on Side gig with the amber GitHub line; the drawer switcher and the New project dropdown show ⚠ next to it. | `3b-…`, `6c-…`, `7b-…` |
| Management → Personas | List with ✓ Rodeo, `draft` Draft, ✓ Personal, ⚠ Side gig and + New persona; right side: name, project count, commit identity (Save buttons), Claude and GitHub status with Re-login, GitHub auth type (gh login / Personal access token), remembered repos, Delete with "Used by N projects". The draft shows "Setup was not finished." with Resume / Delete. | `3a-…`, `3b-…`, `3c-…` |
| Instructions / Agent settings | Instructions: monospace editor with the saved text, Save/Revert, "Applies to agents started after saving." Agent settings: allow and deny rows, env table, Save/Revert; an added row enables Save and Revert, and Revert drops it. | `4-…`, `5-…` |
| Kanban, all personas | Four projects under three personas; each card carries its persona tag (Rodeo, Personal, Side gig). The Persona filter lists the ready personas with counts (Rodeo 2, Personal 1, Side gig 1). Picking Personal shows only its card, without a tag. | `6-…`, `6b-…`, `6d-…` |
| New project | Persona dropdown preset to the active persona (Rodeo, repo chip `webapp`); the draft is not offered; switching to Personal swaps the repo chips to `blog`. | `7-…`, `7b-…`, `7c-…` |
| Projects view | Each row names its persona under the project name. | `8-…` |
| Delete rules | Deleting Rodeo (2 projects) → 409 "the persona still has 2 project(s); archive them first"; deleting the draft → 204 and its folder gone. | — |

## Observations (not fixed)

1. **Personas created outside the app show by id until the app re-fetches.** Personal and Side gig were created
   through the API while the app was open; the Kanban cards were tagged `personal` and `side-gig` (ids) until
   Management was opened, which re-fetched the list; then the names showed (`6-…` is after). `persona_rows` and
   `persona_names` (`crates/fleet-gui/src/app/setup.rs`) fall back to the id for an unknown persona. Only reachable
   today through the API or a second app; the web UI is frozen. A persona id missing from the cached list on a
   snapshot could trigger a re-fetch.
2. **The last row of the persona dropdowns is clipped.** In the drawer switcher (`6c-…`) and in New project's
   Persona dropdown (`7b-…`) the popup's bottom edge cuts through the last row ("Side gig ⚠"), about a third of its
   height lost. The board's Persona filter popup (`6b-…`, plain checkbox rows) is not clipped. Both clipped ones are
   `persona_choice` in `crates/fleet-gui/src/components/persona_filter.rs`, whose rows nest `ui.horizontal` inside
   `list_row`; that the popup is sized before the row's real height is known is a guess, not checked.
3. **The installer still prints the old trust step.** (Fixed since: it says the persona's Claude login trusts
   the folder, and checks for gh 2.40 or newer.) `packaging/install.sh` (lines ~112–120, and the
   `prints_the_one_time_trust_step` case of `test-install.sh`) tells the user to run `claude` in the projects
   folder and trust it. With personas that trusts the folder for `~/.claude` only, which no agent uses; the wizard's
   Claude step now does it per persona. It also does not check that `gh` is installed. Out of this task's files;
   worth a follow-up.
4. **The credential helper is not scoped to the host.** The agent gets `credential.helper` (cleared, then
   `!gh auth git-credential`), not `credential.https://<host>.helper` as the spec words it. Harmless as far as this
   run shows: inherited helpers are cleared either way and `gh` answers only for hosts it is logged in to, which is
   the persona's one host. Noted as a spec drift, not a bug. (Fixed since: the helper is now
   `credential.https://<host>.helper`.)
5. A ⚠ persona can still be picked in New project; the spawn then fails with the "Log in … again" message, as the
   spec asks. Not hidden on purpose, presumably, so the user sees why.
6. `2-…` and `8-…` have the scratch paths painted over (they name the local home).

## Build and tests

- `./build.sh`: rustfmt clean; TS bindings regenerated with no change beyond the tree's existing uncommitted ones;
  web tests 61/61 and build OK; fleet-client, fleet-gui and the 21 installer cases pass; daemon for x86_64 and
  aarch64 and the app (zig, glibc 2.28) in `dist/`.
- `cargo test --workspace`: the first run stopped on one failure in `fleet-client`
  (`tests/login.rs` `pat_login_times_out_without_leaking_the_token`: "cannot run …/gh-slow: Text file busy").
  Re-run with `--no-fail-fast`: 1036 passed, 0 failed. The failure is a test race, not the product: the test
  writes a script and runs it while another test thread in the same binary forks, and the child briefly holds the
  script's write descriptor (ETXTBSY). 1 failure in 30 runs of that test binary.

## Not exercised

- The real `claude` and `gh`, a real host, a PAT typed into the masked field, a terminal opened by Log in…:
  see the checklist below.
- Typing in the window (the wizard's step 2 form, renaming, the Agent settings validation in the form): keyboard
  events did not reach the app under Xvfb without a window manager. The same validation was checked through the API.
- Upgrade from a pre-personas install (projects assigned to the first persona, repos adopted by host), a persona
  folder deleted by hand, URL repos (clone and fetch per persona), and the light theme.

## Real-CLI check for Antonio

On your own desktop, with a throwaway home so your real `~/.config/fleet-manager` and daemon stay as they are.
Use a free port, not 7878.

1. `export T=$(mktemp -d); mkdir -p $T/.config/fleet-manager; printf 'listen = "127.0.0.1:7966"\n' > $T/.config/fleet-manager/config.toml`
   (or skip the config to see the wizard's step 1, then pick folders inside `$T`).
2. Start the new daemon yourself first, so the app does not start an older installed one, then the app:
   `export FLEET_TMUX_SOCKET=fbreal; HOME=$T dist/fleet-daemon-x86_64-linux &` then
   `HOME=$T dist/fleet-gui-x86_64-linux`. Keep your `PATH`, `DISPLAY` and terminal; `HOME=$T` keeps both off your
   real config and Claude/gh folders, and the socket keeps the agents off your real `fleet-manager` tmux server.
3. Wizard step 2: persona `personal`, your noreply email, host `github.com`, auth **PAT**. Step 3: Claude → in the
   terminal trust the folder (Down, Enter), `/login` with your personal Claude account, `/exit`. GitHub → paste the
   fine-grained PAT. Both lines green, Finish.
4. Management → Personas → + New persona: `rodeo`, work email, host `rodeofx.ghe.com`, auth **gh login**. Claude →
   trust + `/login` with the company account. GitHub → `gh auth login` device flow in the terminal; when gh asks to
   authenticate git, say yes (it writes into the persona's own `gitconfig`, not yours).
5. Check: `ls -l $T/.local/share/fleet-manager/personas/*/gh/hosts.yml` exist (mode 600); your own
   `~/.config/gh/hosts.yml` and `~/.gitconfig` unchanged (`git diff --no-index` against a copy taken first, or
   `stat` mtimes). Don't inspect the keyring with `secret-tool search`: it prints the secrets it finds. The
   `hosts.yml` files above already show `--insecure-storage` took effect.
6. New project under `personal` with `https://github.com/TonyHeadband/fleet-manager.git`; one under `rodeo` with a
   work repo on `https://rodeofx.ghe.com/…`. Try the work URL under `personal`: it must be refused.
7. In each agent's session (`tmux -L fbreal attach -t fleet-…`), open a second window with `Ctrl-b c` (it gets
   the session's environment: the persona's variables, and none of the ambient tokens even if the tmux server was
   started from a shell that had them) and, from the repo's worktree:
   `echo $GH_HOST; echo ${GH_TOKEN-unset} ${SSH_AUTH_SOCK-unset}; git config --get-regexp '^credential\.'; git var GIT_AUTHOR_IDENT; gh auth status`
   — expect the persona's host, `unset unset`, `credential.helper ` (empty) then
   `credential.https://<host>.helper !gh auth git-credential`, the persona's name and email, and gh logged in as the
   right account only.
8. `git push --dry-run origin HEAD` from each worktree: succeeds as the right account (personal on github.com,
   work on GHE) with no password prompt and no keyring window.
9. Break one: `mv $T/.local/share/fleet-manager/personas/rodeo/gh/hosts.yml{,.bak}`, Re-login's status turns
   amber after a check, and a new project on `rodeo` fails with "Log in to rodeo's GitHub again". Put it back.
10. Clean up: quit the app, kill the daemon on 7966, `tmux -L fbreal kill-server`, `rm -rf $T`. Revoke the PAT if
    it was made for this test.

## Cleanup

Stopped by pid after checking each process's `HOME` was the scratch one: the app, the daemon, Xvfb `:95`. The
agents' sessions outlive the daemon by design, so the `fbt12` tmux server was killed afterwards (`no server running`).
Port 7965 free afterwards. The scratch homes stay under `.superpowers/` (git-ignored).
