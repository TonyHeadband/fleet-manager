# fleet-manager

A desktop command room for a fleet of coding agents. Start several Claude Code agents on your repos, see at a
glance which ones are working, which are waiting on you and which have finished, and answer, pause, review, stop
or archive them from one window.

Today it is a board you fly by hand. Where it is going (an orchestrator that runs a project on autopilot inside a
policy you set, specialist agents with enforced limits) is in [ROADMAP.md](ROADMAP.md).

> **Status: v0.1, personal project.** Linux only (x86_64 app; daemon on x86_64 and aarch64), Claude Code only,
> loopback only with no authentication. Expect breaking changes.

## What it does

- **Projects.** A project is one agent session working toward a goal, in its own folder under the projects root.
  Each repo it touches gets a git worktree on the project's branch, `fleet/<project>`, so agents never step on
  each other or on your checkouts. Repos can be paths, git URLs or names fleet-manager remembers, and an agent can
  ask for another repo mid-session (you grant or refuse it from the card).
- **A board driven by Claude Code hooks, not by scraping the terminal.** Columns: Queued, Working, Needs you,
  Talking, Review. A card in Needs you says why: a permission prompt, a question, an error, or startup.
- **Act from the card.** Answer, pause, resume, stop. The side panel shows the agent's live terminal (read-only)
  and the diff of every repo in the project.
- **Stop now, archive later.** Stop ends the agent and keeps everything on disk: worktrees, branches, the project
  folder and its record. Archive, from the Projects view, cleans up a stopped project's disk after showing exactly
  what it will delete and what stays; branches are never deleted, and the record stays in the history.
- **Personas: one set of accounts per project.** A persona bundles a Claude login, a GitHub login on one host
  (e.g. `github.com` or a GitHub Enterprise host) and a commit identity. Each project belongs to one persona, chosen
  when it is created, and its agent, clones and fetches use only that persona's logins: nothing comes from
  `~/.claude`, `~/.config/gh` or an ambient token. Repos are remembered per persona, and their remotes must be HTTPS
  on its host (SSH is refused for now, #156). The logins stay inside the `claude` and `gh` CLIs, as plain files in
  the persona's own 0700 folder (an encrypted store is #161); fleet-manager never reads or stores a token.
  Switch the active persona in the drawer; filter the board by persona.
- **Management.** Where the fleet is configured: **Personas** (create, re-login, rename, delete), **Instructions**
  (one Markdown file given to every agent as its `CLAUDE.md`, the replacement for a global one) and **Agent
  settings** (permission allow/deny rules and env merged into every agent's Claude settings).
- **Agents outlive the app.** They run as the interactive `claude` CLI in tmux. Close the window, keep the daemon
  running, come back later; or take over directly with `tmux -L fleet-manager attach -t <session>`.

## How it works

```
fleet-gui (egui)  ──HTTP + SSE──▶  fleet-daemon  ──tmux──▶  claude (one session per project)
                                     ▲     │                     │
                                     │     └── SQLite state      │
                                     └──── hooks + MCP ──────────┘
```

- `fleet-daemon` spawns each agent in a dedicated tmux server, hands it hooks that post status back, and serves
  an MCP endpoint for repo requests. It listens on `127.0.0.1:7878` and keeps state in
  `~/.local/share/fleet-manager`.
- `fleet-gui` is the desktop app. It starts the daemon if none is running and draws the board from a UI schema the
  daemon serves, so columns, fields and actions are defined in one place (`crates/fleet-api/src/ui.rs`).
- The web UI in `web/` is embedded in the daemon. It is frozen at its v1 state until it comes back for remote access.
  It knows nothing of personas, and a project now needs one, so it cannot start projects; use the desktop app.

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
   1. **Folders**: where projects live (default `~/fleet-projects`) and where personas live (default
      `~/.local/share/fleet-manager/personas`). This writes `~/.config/fleet-manager/config.toml`.
   2. **Your first persona**: a name, the commit name and email, and the GitHub host.
   3. **Log in**: **Claude** opens a terminal running `claude` in the projects folder with the persona's own
      config folder. Trust the folder first (choose **"Yes, I trust this folder"**: Down, then Enter; the dialog
      starts on "No, exit"), then type `/login`. **GitHub** opens `gh auth login` for the persona's host, or takes
      a personal access token that the app pipes to `gh` once. Either way `gh` keeps the token in the persona's
      folder (`--insecure-storage`), not the system keyring. Each line shows the account (green) once the CLI
      reports a login; **Finish** is enabled when both do.
3. **New project**: give it a goal, pick the persona (the active one by default) and, optionally, repos. The card
   appears and the agent starts.

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
| `projects_root` | `~/fleet-projects` | One folder per project |
| `state_dir` | `~/.local/share/fleet-manager` | Database, `daemon.log`, clones of URL repos |
| `branch_prefix` | `fleet` | Agent branches are `<prefix>/<project>` |
| `claude_command` | `["claude"]` | How agents are started |
| `personas_dir` | `~/.local/share/fleet-manager/personas` | One folder per persona: its `claude` and `gh` logins (mode 700) |
| `gh_command` | `gh` | The GitHub CLI |

The agent instructions and settings edited in Management live next to the config, in
`~/.config/fleet-manager/agent/` (`instructions.md`, `settings.json`).

## Development

```bash
cargo test --workspace          # daemon tests need tmux and git on PATH
pnpm --dir web test
bash packaging/test-install.sh
```

- Design notes: [docs/concept.md](docs/concept.md) (what fleet management means here), [docs/ui.md](docs/ui.md)
  (how both UIs are put together). Specs and plans live in `docs/superpowers/`.
- Deferred work: [GitHub issues](https://github.com/TonyHeadband/fleet-manager/issues).
- Commits follow [Conventional Commits](https://www.conventionalcommits.org/).

## License

[Apache-2.0](LICENSE)
