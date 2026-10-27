# fleet-manager

A desktop command room for a fleet of coding agents. Start several Claude Code agents on your repos, see at a
glance which ones are working, which are waiting on you and which have finished, and answer, pause, review, stop
or archive them from one window.

A task can be one agent you fly by hand, or an orchestrator that breaks the goal into steps and launches
specialist agents for you to approve and watch. Where it is going (autopilot inside a policy you set, enforced
specialist limits) is in [ROADMAP.md](ROADMAP.md).

> **Status: v0.1, personal tool.** Linux only (x86_64 app; daemon on x86_64 and aarch64), Claude Code only,
> loopback only with no authentication. Expect breaking changes.

## What it does

- **Tasks.** A task is one agent session working toward a goal, in its own folder under the tasks root.
  Each repo it touches gets a git worktree on the task's branch, `fleet/<task>`, so agents never step on
  each other or on your checkouts. Repos can be paths, git URLs or names fleet-manager remembers, and an agent can
  ask for another repo mid-session (you grant or refuse it from the card).
- **Orchestrated tasks.** Pick a Type when you create a task: an orchestrator agrees the goal with you,
  proposes a breakdown you approve in the side panel, launches specialist agents, merges their branches and opens
  one PR after the repo's tests pass. The board shows the task as one group with a rolled-up status. See
  [Orchestrated tasks](#orchestrated-tasks).
- **A board driven by Claude Code hooks, not by scraping the terminal.** Columns: Queued, Working, Needs you,
  Talking, Review. A card in Needs you says why: a permission prompt, a question, an error, or startup.
- **Act from the card.** Answer, pause, resume, stop. The side panel shows the agent's live terminal (read-only)
  and the diff of every repo in the task.
- **Stop now, archive later.** Stop ends the agent and keeps everything on disk: worktrees, branches, the task
  folder and its record. Archive, from the Tasks view, cleans up a stopped task's disk after showing exactly
  what it will delete and what stays; branches are never deleted, and the record stays in the history.
- **Personas: one set of accounts per task.** A persona bundles a Claude login, a GitHub login on one host
  (e.g. `github.com` or a GitHub Enterprise host) and a commit identity. Each task belongs to one persona, chosen
  when it is created, and its agent, clones and fetches use only that persona's logins: nothing comes from
  `~/.claude`, `~/.config/gh` or an ambient token. Repos belong to products, not personas; a repo's remote must be
  HTTPS on the host of the persona that uses it (SSH is refused for now, #156). The logins stay inside the `claude`
  and `gh` CLIs, as plain files in the persona's own 0700 folder (an encrypted store is #161); fleet-manager never
  reads or stores a token.
  Switch the active persona in the drawer; filter the board by persona.
- **Management.** Where the fleet is configured: **Products** (name, repos, and each repo's specialist roster),
  **Personas** (create, re-login, rename, delete), **Instructions** (one Markdown file given to every agent as its
  `CLAUDE.md`, the replacement for a global one), **Agent settings** (permission allow/deny rules and env merged
  into every agent's Claude settings), and **Orchestrators**, **Specialists** and **Skills** (built-in and user
  definitions, edited as Markdown).
- **Agents outlive the app.** They run as the interactive `claude` CLI in tmux. Close the window, keep the daemon
  running, come back later; or take over directly with `tmux -L fleet-manager attach -t <session>`.

## Orchestrated tasks

A task of a chosen Type (`direct`: one step; `structured`: brainstorm, plan, one specialist per task, a review
step) instead of Solo.

1. New task: pick the Type, the persona and one repo; the roster (the specialists it may use) comes from the
   repo.
2. You talk to the orchestrator in its terminal until the goal is agreed.
3. It proposes a breakdown; the task goes to Needs you. Approve it or request changes in the side panel.
4. It launches one specialist per step. Each is a normal agent on the board with its own worktree, on a sub-branch
   of the task branch; the orchestrator merges each finished step back.
5. When the steps are done it calls `open_pr`: the daemon runs the checks the repo's workflow file lists for the
   change, and only if they pass pushes the task branch and opens the PR. The default stop is an open PR; nothing is merged.

Every question and permission prompt still reaches you; there is no autopilot yet. A repo needs a
**`.fleet/workflow.yml`** on its base branch on `origin` before an orchestrated task can use it: it lists the
checks the gate runs before a pull request, and is fetched when the task is created. Orchestrators and specialists cannot spawn
in-chat subagents, and an orchestrated task has exactly one repo.

Pushes and PRs by the agents themselves are blocked on a best-effort basis, not as a security boundary: text deny
rules on `git push`, `gh pr create` and `gh api`, plus an env block that points `origin`'s push URL nowhere. They
stop the model's ordinary attempts. Known bypasses: `git push <url>` hidden from the text rules (in `bash -c` or a
script), `env -u GIT_CONFIG_COUNT git push`, and a repo whose own config sets `remote.origin.pushurl`. `open_pr` is
the only push the daemon makes, and only after the tests pass.

Definitions live in the binary (built-ins, read-only) plus your files in `~/.config/fleet-manager/fleet/`
(`orchestrators/`, `specialists/`, `skills/`); a user file with a built-in's name overrides it.

## How it works

```
fleet-gui (egui)  ──HTTP + SSE──▶  fleet-daemon  ──tmux──▶  claude (one session per task)
                                     ▲     │                     │
                                     │     └── SQLite state      │
                                     └──── hooks + MCP ──────────┘
```

- `fleet-daemon` spawns each agent in a dedicated tmux server, hands it hooks that post status back, and serves an
  MCP endpoint for repo requests and, for orchestrated tasks, the breakdown, spawn and `open_pr` tools. It
  listens on `127.0.0.1:7878` and keeps state in `~/.local/share/fleet-manager`.
- `fleet-gui` is the desktop app. It starts the daemon if none is running and draws the board from a UI schema the
  daemon serves, so columns, fields and actions are defined in one place (`crates/fleet-api/src/ui.rs`).
- The web UI in `web/` is embedded in the daemon. It is frozen at its v1 state until it comes back for remote access.
  It knows nothing of personas, and a task now needs one, so it cannot start tasks; use the desktop app.

| Crate | Job |
|---|---|
| `fleet-api` | The client/daemon contract: types only, exported to TypeScript for the web UI |
| `fleet-daemon` | Agents, tmux, hooks, MCP, git worktrees, SQLite store, HTTP API |
| `fleet-client` | Everything a Rust front end needs: config, HTTP, event stream, starting and stopping the daemon |
| `fleet-gui` | The egui desktop app |
| `fleet-fs` | Filesystem and git queries behind the folder picker |

## Requirements

- Linux with glibc 2.28 or newer (Rocky/RHEL 8, Fedora, Nobara, …)
- tmux 3.2 or newer
- [Claude Code](https://docs.claude.com/en/docs/claude-code) (`claude`); you log in once per persona from the app
- [GitHub CLI](https://cli.github.com/) (`gh`) 2.40 or newer, for each persona's GitHub login
- git
- A terminal emulator (`$TERMINAL`, `x-terminal-emulator`, `gnome-terminal`, `konsole`, `xfce4-terminal` or
  `xterm`), where the app opens the `claude` and `gh` logins

## Build

```bash
./build.sh
```

Builds the web UI, embeds it in the daemon, runs the client, app and installer tests, and writes the binaries and
the installer to `dist/`. Needs Rust with `cargo-zigbuild`, a zig (`python3 -m venv ~/.cache/fleet-manager/zig-venv
&& ~/.cache/fleet-manager/zig-venv/bin/pip install ziglang`), Node 20+ and pnpm. Binaries are linked against
glibc 2.28 through zig, so one build runs on every distribution above it.

Each component also builds alone with `scripts/web.sh`, `scripts/daemon.sh`, `scripts/gui.sh` or
`scripts/package.sh`; `build.sh` runs `cargo fmt --all --check` first, then the scripts in that order.
`.fleet/workflow.yml` lists the checks the Fleet app's pull-request gate runs on a branch, and which changed paths
trigger each. The `web` step needs the npm registry (`pnpm install`), so offline or air-gapped, a change that falls
back to every step needs a local registry mirror.

## Install

```bash
dist/packaging/install.sh            # or: --prefix DIR (default ~/.local)
```

Installs `fleet-daemon` and `fleet-gui` to `~/.local/bin`, plus a menu entry and icon. It writes no config and
touches nothing outside the prefix. Running it again upgrades in place.

## First run

1. Launch **fleet-manager** from the application menu, or run `fleet-gui`.
2. The setup wizard runs whenever no persona exists (also after upgrading an install from before personas):
   1. **Folders**: where tasks live (default `~/fleet-tasks`) and where personas live (default
      `~/.local/share/fleet-manager/personas`). This writes `~/.config/fleet-manager/config.toml`.
   2. **Your first persona**: a name, the commit name and email, and the GitHub host.
   3. **Log in**: **Claude** opens a terminal running `claude` in the tasks folder with the persona's own
      config folder. Trust the folder first (choose **"Yes, I trust this folder"**: Down, then Enter; the dialog
      starts on "No, exit"), then type `/login`. **GitHub** opens `gh auth login` for the persona's host, or takes
      a personal access token that the app pipes to `gh` once. Either way `gh` keeps the token in the persona's
      folder (`--insecure-storage`), not the system keyring. Each line shows the account (green) once the CLI
      reports a login; **Finish** is enabled when both do.
3. **New product**: Management → Products → **+ New product**. Give it a name and add its repos (a local path, a
   URL, or both).
4. **New task**: give it a goal, pick the persona (the active one by default) and, optionally, repos of the
   product. The card appears and the agent starts.

More personas: Management → Personas → **+ New persona** runs steps 2 and 3. A persona whose login breaks later is
flagged in the drawer and in Management, and spawning on it fails with "Log in to … again"; nothing falls back to
your own login. To give a persona a Claude plugin, run `claude` with `CLAUDE_CONFIG_DIR` set to its `claude` folder.

Agents only reach Needs you when Claude Code asks before running tools. If your own Claude settings default to a
mode that never asks, set `claude_command = ["claude", "--permission-mode", "default"]` in the config.

How branches and merging work: [docs/usage.md](docs/usage.md).

## Configuration

`~/.config/fleet-manager/config.toml`, read by both the daemon and the app. Every key is optional; see
[packaging/config.example.toml](packaging/config.example.toml).

| Key | Default | |
|---|---|---|
| `listen` | `127.0.0.1:7878` | Loopback only |
| `tasks_root` | `~/fleet-tasks` | One folder per task |
| `state_dir` | `~/.local/share/fleet-manager` | Database, `daemon.log`, clones of URL repos |
| `branch_prefix` | `fleet` | Agent branches are `<prefix>/<task>` |
| `claude_command` | `["claude"]` | How agents are started |
| `personas_dir` | `~/.local/share/fleet-manager/personas` | One folder per persona: its `claude` and `gh` logins (mode 700) |
| `gh_command` | `gh` | The GitHub CLI |

The agent instructions and settings edited in Management live next to the config, in
`~/.config/fleet-manager/agent/` (`instructions.md`, `settings.json`); orchestrator, specialist and skill
definitions in `~/.config/fleet-manager/fleet/`.

## Development

```bash
cargo test --workspace          # daemon tests need tmux and git on PATH
pnpm --dir web test
bash packaging/test-install.sh
```

- Design notes: [docs/concept.md](docs/concept.md) (what fleet management means here), [docs/ui.md](docs/ui.md)
  (how both UIs are put together).
- Deferred work: [GitHub issues](https://github.com/TonyHeadband/fleet-manager/issues).
- Commits follow [Conventional Commits](https://www.conventionalcommits.org/).

## License

[Apache-2.0](LICENSE)
