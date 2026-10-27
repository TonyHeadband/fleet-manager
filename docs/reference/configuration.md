# Configuration

`~/.config/fleet-manager/config.toml`, read by both the daemon and the app. Every key is optional; see
`packaging/config.example.toml` in the repo.

| Key | Default | |
|---|---|---|
| `listen` | `127.0.0.1:7878` | Loopback only |
| `tasks_root` | `~/fleet-tasks` | One folder per task |
| `state_dir` | `~/.local/share/fleet-manager` | Database, `daemon.log`, clones of URL repos, each product's `internal/` notes repo |
| `branch_prefix` | `fleet` | Agent branches are `<prefix>/<task>` |
| `claude_command` | `["claude"]` | How agents are started |
| `tmux_command` | `tmux` | The tmux program, when the one on `PATH` is missing or older than 3.2 |
| `personas_dir` | `~/.local/share/fleet-manager/personas` | One folder per persona: its `claude` and `gh` logins (mode 700) |
| `gh_command` | `gh` | The GitHub CLI |
| `test_timeout_secs` | `1800` | How long the checks `open_pr` runs may take, all steps together |
| `max_breakdown_versions` | `8` | Most breakdown versions one task may accumulate. Applies after the daemon restarts |
| `max_steps` | `20` | Most steps one breakdown may declare. Applies after the daemon restarts |
| `max_parallel_specialists` | `3` | Most specialists one orchestrator may run at once. Applies after the daemon restarts |

The agent instructions and settings edited in Management live next to the config, in
`~/.config/fleet-manager/agent/` (`instructions.md`, `settings.json`); orchestrator, specialist and skill
definitions in `~/.config/fleet-manager/fleet/`.
