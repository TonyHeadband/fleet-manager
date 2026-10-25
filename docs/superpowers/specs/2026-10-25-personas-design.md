# Personas: one set of accounts per project — design

Date: 2026-10-25. Scope: `fleet-api`, `fleet-daemon`, `fleet-client`, `fleet-gui`. The web UI stays frozen.
Builds on the app shell (`2026-10-24-app-shell-design.md`: drawer, Settings tabs) and the projects-first model
(`2026-10-24-projects-first-design.md`: a project is one agent session). Line references are from 2bc6791.
Mockup: `.superpowers/brainstorm/3224657-1792904458/content/personas-v1.html` (gitignored; wireframe only).

## Why

Antonio works with two GitHub accounts on one machine: the personal one on github.com and the work one on
`rodeofx.ghe.com`. Today fleet-manager has no notion of either. Agents inherit the daemon's environment
(`tmux new-session -e …`, `crates/fleet-daemon/src/tmux.rs:122`), clones and fetches run plain `git`
(`crates/fleet-daemon/src/git.rs:40`), so every project uses whatever login happens to be ambient. Keeping the
personal and work accounts apart is done by hand, with instructions in `CLAUDE.local.md`.

The same goes for Claude: work projects should run on the company's Claude account, personal ones on Antonio's.

fleet-manager is also meant to be pushed to colleagues. A persona is the first thing a new user sets up, and it is
the entry point for remote and multi-user later (ROADMAP Phase 2 and 3), without any remote capability now.

## Model

- **Person**: the human. For now the local OS user; `Agent.owner` stays `$USER` (`crates/fleet-api/src/model.rs:62`).
  Later the person is what logs in remotely and owns several personas. Nothing new is built for it in this round.
- **Persona**: a named bundle of accounts, e.g. `rodeo`, `personal`. It holds:
  - a **Claude** login (its own `CLAUDE_CONFIG_DIR`),
  - a **GitHub** login on exactly one host (its own `GH_CONFIG_DIR`, host e.g. `github.com` or `rodeofx.ghe.com`),
  - a **commit identity** (name, email).
  Room for more connections later (other providers, Slack…), but only Claude and GitHub now.
  Each persona is a **complete sandbox** (Antonio): nothing is shared with `~/.claude`, `~/.config/gh` or another
  persona. What every agent should get regardless of persona lives in fleet-manager (see Fleet-wide agent config).
- **Project**: owned by exactly one persona, chosen at creation, never changed afterwards.
- **Remembered repo**: belongs to a persona. The same URL may be remembered by two personas; each gets its own base
  clone, fetched with its own credentials. **Strict host rule** (Antonio): a persona accepts only HTTPS URLs on
  its own GitHub host. Another host, or an SSH URL (`git@…`, `ssh://…`), is refused at resolve time
  (`crates/fleet-daemon/src/repos.rs` `resolve`) with a message naming the persona and host it needs. A path repo
  (the user's own checkout) is checked the same way on its `origin` URL, and belongs to one persona only
  (a path's `repos.location` is unique across personas; a URL's only within one persona); adding it under a second persona is refused with the owner's name.
  (Claude's call for path repos.)

## Goals

### Secrets stay in the CLIs

Hard rule from the roadmap: never read, store or reuse Claude login tokens outside the `claude` CLI. This design
applies the same rule to GitHub. fleet-manager stores only folder paths, names and the commit identity.

- Each persona has a folder under the **personas folder** (default `~/.local/share/fleet-manager/personas`,
  config key `personas_dir` in `config.toml`): `<personas_dir>/<persona>/claude` and `<personas_dir>/<persona>/gh`,
  mode 700.
- **Logging in** opens a terminal running the real CLI with that folder: `claude` (then `/login`) with
  `CLAUDE_CONFIG_DIR=…/claude`, and `gh auth login --hostname <host>` with `GH_CONFIG_DIR=…/gh`.
- **GitHub auth type is the user's choice** (Antonio), per persona:
  - **gh login**: the interactive `gh auth login` above (browser/device flow).
  - **PAT**: the user pastes a token in a masked field. fleet-manager pipes it once on stdin to
    `gh auth login --hostname <host> --with-token` with the persona's `GH_CONFIG_DIR`, then drops it from memory.
    It is never written by fleet-manager, logged, put in argv or env, or sent to the daemon over HTTP: the GUI
    runs the `gh` command itself. From then on both types look the same: `gh` holds the token, agents use
    `gh auth git-credential`. (Claude's call: the pipe-into-gh approach, to keep the no-token anti-goal.)
  - Management → Personas shows the auth type and lets Re-login switch it.
- **Where `gh` keeps the token** (Antonio): in the persona folder (`gh auth login --insecure-storage`, so
  `<persona>/gh/hosts.yml` in the 700 folder), not the system keyring. The keyring is shared by every persona on the
  same host and the user's own `gh`, and an agent's push could pop a keyring unlock window. This matches how `claude`
  keeps its login in its config dir. The interactive login also sets `GIT_CONFIG_GLOBAL=<persona>/gh/gitconfig`, so
  gh's "Authenticate Git?" step never edits the user's `~/.gitconfig`.
- **Checking a login** runs the CLI, never reads the files: `gh auth status --hostname <host>` with `GH_CONFIG_DIR`;
  for Claude, a status check through the `claude` CLI with `CLAUDE_CONFIG_DIR` (the builder picks the cheapest
  command that reports logged-in without spending tokens).

### What an agent gets

When a project starts (and whenever the daemon runs git for it: clone, fetch, worktree), the persona's environment
is applied:

- `CLAUDE_CONFIG_DIR=<persona>/claude`
- `GH_CONFIG_DIR=<persona>/gh`, `GH_HOST=<host>`
- git credentials through `gh`: `credential.<https://host>.helper` set to `!gh auth git-credential`, with the
  default helpers cleared first (the same trick as `CLAUDE.local.md`'s push command), passed as
  `GIT_CONFIG_COUNT/KEY/VALUE` env so no file is written into the user's checkouts
- `GIT_AUTHOR_NAME/EMAIL`, `GIT_COMMITTER_NAME/EMAIL` from the persona

The daemon's own environment is never mutated; each command gets the persona env explicitly.

### Fleet-wide agent config

Because personas share nothing, the things that used to come from `~/.claude` for every agent come from
fleet-manager instead (Antonio). This round, two pieces, stored under the daemon's config dir and edited in the
new **Management** view (below):

- **Agent instructions**: one Markdown file applied to every agent, the replacement for a global `CLAUDE.md`. The
  provider-neutral route is an instructions file in the project folder (`CLAUDE.md` for Claude Code, `AGENTS.md`
  later), written at spawn next to `PROMPT.md`.
- **Agent settings**: permissions and env, merged into the per-agent `--settings` file the daemon already writes
  (`crates/fleet-daemon/src/hooks_cfg.rs`). fleet's own hooks always win on conflict.

Skills and plugins are **not** in this round (Antonio): they arrive with ROADMAP Phase 1 step 3 (Skills). Until
then a user who needs a plugin installs it per persona by running `claude` with that persona's
`CLAUDE_CONFIG_DIR`.

### Management view

A new drawer nav entry, **Management** (Antonio): where the fleet itself is configured, as opposed to Settings
(the app and the daemon). Nav order: Kanban, separator, Management, Settings. No Detail column. It is the home for
the Phase 1 fleet pieces later (specialists, skills, policies), each as a tab. This round, three tabs, in order
**Personas**, **Instructions**, **Agent settings** (Personas sits here, not in Settings: Antonio):

- **Instructions**: a monospace multi-line editor over the agent instructions file, with Save and Revert, and a note
  that it applies to agents started after saving.
- **Agent settings**: a structured form, not raw JSON: permission `allow` and `deny` rule lists (add/remove rows)
  and an `env` key/value table. Validated on Save (rule syntax, no env keys fleet sets itself: `CLAUDE_CONFIG_DIR`,
  `GH_*`, `GIT_*`, the MCP hold vars). (Claude's call: a form keeps the merge with fleet's own settings safe.)

The daemon owns the files: the GUI reads and writes them through new routes, so the web UI can reuse them later.

### Claude trust per persona

Claude's folder trust lives in the Claude config dir. Projects inherit trust from the trusted projects root
(`crates/fleet-daemon/src/agents.rs:432`), so the root must be trusted **once per persona**. The persona's Claude
login step also runs the trust step for the projects root with that `CLAUDE_CONFIG_DIR` (reusing
`trust_command`, `crates/fleet-gui/src/shell.rs:664`). A persona is not usable until its root is trusted.

### Setup wizard (first run)

Replaces today's "choose projects folder" first run. Three steps:

1. **Folders**: projects folder (as today) and personas folder.
2. **First persona**: name, commit name, commit email, GitHub host.
3. **Log in**: Claude (+ trust the projects root) and GitHub, each a "Log in…" button that opens the terminal, then
   a live ✓ / not logged in from the status check. **Finish is disabled until both are ✓.**

It runs whenever no persona exists, including on upgrade of an existing install. On upgrade:

- Existing projects are assigned to the first persona created (Antonio).
- Existing remembered repos go to the persona whose host matches their URL; repos with no matching persona yet
  (another host, SSH) stay stored but hidden and unusable until a matching persona is created, then attach to it.
  (Claude's call: avoids handing work repos to the personal persona.)
- Existing base clones stay where they are (`<state_dir>/repos/<name>`), so existing worktrees need no repair;
  only new clones go under the per-persona folder. (Claude's call.)

### Management → Personas

The first Management tab. A list of personas on the left, `+ New persona` at the bottom; on the right,
the selected persona: name, project count, commit identity, Claude status + Re-login, GitHub host + account +
Re-login, its remembered repos. New persona runs steps 2–3 of the wizard.

A persona is created in the daemon at step 2 (it needs an id for its folders) with `ready = false`. It cannot be
picked anywhere (drawer, New project, spawn) until both logins and the trust step are ✓. Cancelling the wizard or
New persona deletes the draft and its folder; a draft left by a crash shows in Management → Personas with Resume / Delete.
(Claude's call.)

Deleting a persona: only when it has no projects; removes its folder after a confirm. Renaming: changes the
display name only; the folder name (an id) does not change.

### Choosing and seeing personas

- **Active persona** in the drawer header: a switcher. New projects default to it.
- **New project dialog**: a `Persona` dropdown above the repo picker, preset to the active persona, overridable
  for this one project. The repo picker shows only that persona's remembered repos.
- **Board filter**: a `Persona` dropdown in the Kanban header next to the repo filter, same behaviour as
  `repo_filter` (`crates/fleet-gui/src/components/repo_filter.rs`): "All personas" or one or several personas.
  Independent of the active persona. When more than one persona is visible, each card shows a persona tag.

### Failure modes

- **Login broken later** (expired, revoked): spawning on that persona fails fast with "Log in to <persona>'s
  GitHub/Claude again" and a link to Management → Personas. The persona shows a warning in the drawer switcher and in
  Management. Running projects carry on until their own calls fail; nothing is killed.
- **Persona folder missing** (deleted by hand): same as a broken login.
- **Wrong host or SSH URL**: refused when the repo is resolved (see the strict host rule), before anything is
  created. git runs with only the persona's credential helper (all inherited helpers cleared), so a URL that
  slipped through fails instead of using ambient credentials.

## API

- `SpawnRequest` gains `persona: String` (required; the daemon rejects an unknown or not-ready persona).
- `Agent` gains `persona: String` (`#[serde(default)]` so existing rows load; upgrade fills it with the first
  persona).
- New routes for personas: list (with readiness), create, update (rename, identity), delete, status refresh. The
  login itself is a GUI action (opening a terminal), not a daemon route.
- Repo routes take or return the persona.

## Storage

- `personas` table (migration `004_personas.sql`): id, display name, GitHub host, commit name, commit email,
  created_at. Folders derive from id.
- `repos`: add a nullable `persona` column (null = upgraded repo with no matching persona yet); the key becomes
  (persona, name); `location` is unique per persona, and a path location across all personas. New base clones live under `<state_dir>/repos/<persona>/<name>`.
- Fleet-wide agent instructions and settings: files under the daemon's config dir, not SQLite.

## Non-goals

- Remote access, auth on the API, a hub, multiple people on one daemon.
- More than one GitHub host per persona; GitLab or other forges.
- Other agent runtimes or API-key providers (ROADMAP horizon).
- Moving a project to another persona.
- SSH remotes (refused, not isolated); SSH key per persona is later.
- Skills, plugins, user MCP servers shared across personas (Phase 1 Skills).

## Anti-goals

- fleet-manager reading, copying or storing a token, Claude's or GitHub's.
- A project quietly running on the wrong account: a missing or broken persona fails loudly, never falls back to
  the ambient login.
- Writing persona credentials or identity into the user's own checkouts' `.git/config`.

## Left to the builder

- Terminal for "Log in…": which terminal emulator to launch, or a tmux window on the fleet tmux server.
- The exact Claude status check.
- Layout details of the wizard and the Management tabs, within the app shell's conventions.
- Whether the active persona and board filter persist across restarts (`gui.toml`).
