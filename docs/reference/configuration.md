# Configuration

`config.toml` in the config folder (`~/.config/fleet-manager` on Linux, `%USERPROFILE%\AppData\Roaming\fleet-manager`
on Windows; see [Where your data lives](../getting-started/install.md#where-your-data-lives)), read by both the
daemon and the app. The setup wizard writes `tasks_root` and `personas_dir`; every key is optional. See
`packaging/config.example.toml` in the repo. A leading `~` means your home folder on both systems.

| Key | Default | |
|---|---|---|
| `listen` | `127.0.0.1:7878` | Loopback only |
| `tasks_root` | `~/fleet-tasks` | One folder per task |
| `state_dir` | the data folder | Database, `daemon.log`, clones of URL repos, each product's `internal/` notes repo, `api-token` (the key the Tauri app must show the daemon, new at each start) |
| `branch_prefix` | `fleet` | Agent branches are `<prefix>/<task>` |
| `claude_command` | `["claude"]` | How agents are started. It may not contain `--permission-mode` or `--effort`; the daemon refuses to start if it does |
| `default_mode` | `"auto"` | The permission mode agents start with: `auto`, `acceptEdits`, `manual` or `dontAsk`. `manual` makes agents ask before running tools. Applies to solo agents and to definitions without a `mode:`; the built-in roles set `mode: auto`, so make orchestrated agents ask with a product override `modes.<definition name> = "manual"` |
| `default_effort` | `"medium"` | The effort agents start with: `low`, `medium`, `high`, `xhigh` or `max` |
| `personas_dir` | `personas` in the data folder | One folder per persona: its `claude` and `gh` logins (mode 700 on Linux) |
| `gh_command` | `gh` | The GitHub CLI |
| `test_timeout_secs` | `1800` | How long the checks `open_pr` runs may take, all steps together |
| `max_breakdown_versions` | `8` | Most breakdown versions one task may accumulate. Applies after the daemon restarts |
| `max_steps` | `20` | Most steps one breakdown may declare. Applies after the daemon restarts |
| `max_parallel_specialists` | `3` | Most specialists one orchestrator may run at once. Applies after the daemon restarts |
| `max_pr_attempts` | `3` | Most times one task may try to open its pull request. Applies after the daemon restarts |
| `max_concurrent_tasks` | none | Most orchestrated tasks one product may have working at once; none by default. A product can override it. Applies after the daemon restarts |
| `max_active_products` | none | Most products that may have orchestrated tasks working at once; none by default. Global only. Applies after the daemon restarts |
| `pr_style` | `"plain"` | The PR style pull requests follow: `conventional-semver-pre1.0`, `conventional-semver`, `conventional`, `minimal`, `plain` or one of yours. A style's optional `commits` regex is the pattern every commit subject on the task branch must match: specialists are told it and `open_pr` refuses a branch that misses it. The `conventional` styles set it, `minimal` and `plain` do not. See [Pull request rules](../guide/orchestrated-tasks.md#pull-request-rules) |
| `pr_attribution` | `""` | One line (or lines) the pull request body must end with. Empty adds and checks nothing |
| `pr_draft` | `false` | Open pull requests as drafts |
| `pr_labels` | `[]` | Labels put on pull requests |
| `pr_gate_build` | `"shared"` | `shared`: the `open_pr` tests build in the task's folder, reusing the agents' build. `clean`: they build in a fresh `gate-build` folder that is removed after |

The data folder is `~/.local/share/fleet-manager` on Linux and `%USERPROFILE%\AppData\Local\fleet-manager` on
Windows. Changes to the file apply when the daemon restarts; folders changed in the setup wizard offer
**Restart now**.

`claude`, `gh` and `git` are looked up on the `PATH` the daemon builds when it checks your tools: its own, plus the
system `PATH` (the registry on Windows, a login shell on Linux), so a tool installed after the app started is found
by **Check again** without a restart.

Next to `config.toml` in the config folder: `gui.toml` (the app's own settings, such as the active product and the
close choice), `agent/` (`instructions.md`, `settings.json`, edited in Management) and `fleet/` (orchestrator,
specialist, skill and PR style definitions).
