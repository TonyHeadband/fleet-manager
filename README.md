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
- [Claude Code](https://docs.claude.com/en/docs/claude-code) (`claude`), logged in
- git

## Build

```bash
./build.sh
```

Builds the web UI, embeds it in the daemon, runs the client, app and installer tests, and writes the binaries and
the installer to `dist/`. Needs Rust with `cargo-zigbuild`, a zig (`python3 -m venv ~/.cache/fleet-manager/zig-venv
&& ~/.cache/fleet-manager/zig-venv/bin/pip install ziglang`), Node 20+ and pnpm. Binaries are linked against
glibc 2.28 through zig, so one build runs on every distribution above it.

## Install

```bash
dist/packaging/install.sh            # or: --prefix DIR (default ~/.local)
```

Installs `fleet-daemon` and `fleet-gui` to `~/.local/bin`, plus a menu entry and icon. It writes no config and
touches nothing outside the prefix. Running it again upgrades in place.

## First run

1. Launch **fleet-manager** from the application menu, or run `fleet-gui`.
2. Choose where projects live (default `~/fleet-projects`). This writes `~/.config/fleet-manager/config.toml`.
3. Trust that folder once, so agents start without Claude Code's trust dialog:
   ```bash
   mkdir -p ~/fleet-projects && cd ~/fleet-projects && claude
   ```
   Choose **"Yes, I trust this folder"** (the second option: Down, then Enter) and type `/exit`. The dialog starts
   on "No, exit", so Enter alone quits instead.
4. **New project**: give it a goal and, optionally, repos. The card appears and the agent starts.

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
