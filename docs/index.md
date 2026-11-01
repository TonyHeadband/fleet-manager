# Fleet Manager

*Your agents, on a board, not behind a wall of text.*

A desktop command room for a fleet of coding agents.

<video src="assets/demo.webm" poster="assets/board.png" autoplay loop muted playsinline
       aria-label="Fleet Manager: agents moving across the board"></video>

- **Fly by hand.** Answer, pause and review each agent from its card, with its live terminal and diff one click
  away.
- **Autopilot.** An orchestrator agrees the goal with you, breaks it down and runs specialists under a policy you
  set.
- **Specialists.** Narrow agents with one job each, whose limits Fleet Manager enforces, not only their prompt.

Today you fly by hand, and orchestrated tasks run with you approving each step. Autopilot and enforced limits are
next: see `ROADMAP.md` in the [repository](https://github.com/TonyHeadband/fleet-manager).

## Getting started

- [Install](getting-started/install.md): download, run, update; where your data lives. Windows and Linux.
- [First run](getting-started/first-run.md): the setup wizard and tool check, your first product and your first task.

## Guide

- [The board](guide/board.md): columns, why a card needs you, and what you can do from it.
- [Products and releases](guide/products-and-releases.md): group tasks by product and track a release.
- [Tasks and repos](guide/tasks-and-repos.md): worktrees, branches and merging.
- [Orchestrated tasks](guide/orchestrated-tasks.md): an orchestrator, its specialists and one pull request.
- [Stop now, archive later](guide/stop-and-archive.md): end an agent, clean up its disk later.
- [Personas](guide/personas.md): one set of Claude and GitHub accounts per task.
- [Management](guide/management.md): products, personas, instructions, agent settings and definitions.
- [Fleet Manager on Tauri (preview)](guide/tauri-preview.md): build and run the next app on Linux.

## Reference

- [Configuration](reference/configuration.md): every key of `config.toml`.
- [Architecture](reference/architecture.md): the daemon, the app, the agents and the crates.

## Contributing

- [Development](contributing/development.md): setup, builds, tests, design notes and commits.
- [UI internals](contributing/ui-internals.md): how the web UI and the desktop app are put together.
- [Demo stand-ins](contributing/demo.md): the scripted claude, gh and GitHub behind the smoke test and the demo media.
- [Release checklist](contributing/acceptance/gui.md): what to check on each system before tagging a release.

## Background

- [What fleet management means here](concept.md): the v1 framing of the problem.
