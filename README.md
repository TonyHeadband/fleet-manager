# fleet-manager

*Your agents, on a board, not behind a wall of text.*

A desktop command room for a fleet of coding agents.

![Linux](https://img.shields.io/badge/platform-Linux-informational)
![Claude Code](https://img.shields.io/badge/agents-Claude%20Code-d97757)
![Version](https://img.shields.io/badge/version-0.1.0-blue)
![License](https://img.shields.io/badge/license-Apache--2.0-green)

![fleet-manager: agents moving across the board](docs/assets/demo.gif)

- **Fly by hand.** Answer, pause and review each agent from its card, with its live terminal and diff one click
  away.
- **Autopilot.** An orchestrator agrees the goal with you, breaks it down and runs specialists under a policy you
  set.
- **Specialists.** Narrow agents with one job each, whose limits fleet-manager enforces, not only their prompt.

Today you fly by hand, and orchestrated tasks run with you approving each step. Autopilot and enforced limits are
next: see [ROADMAP.md](ROADMAP.md).

## The problem

Coding agents no longer just answer. They plan, hand work to subagents, review each other and loop, and all of it
happens in one scrolling chat per terminal. With a few of them running you cannot see who is working, who is
stuck and who is waiting on you. Your attention is the scarce resource, and a wall of text spends it badly.

## The solution

Every agent, subagents included, is a card on a board. Its status is live, from Claude Code hooks, not from
scraping the terminal: Working, Needs you (and why), Review. You look at the board, not at the terminals, and
answer the card that needs you.

**New to agents?** Use the board. Give a task a goal, watch its card, answer when it asks.

**Already running agents?** The structure is underneath: products and releases to group the work, an
orchestrator that breaks a task into steps for specialists, a git worktree per task so agents never collide,
personas that keep each task on its own Claude and GitHub accounts, limits enforced by the daemon, and an audit
trail of every task.

## Quick start

Requirements: Linux (glibc 2.28+), tmux 3.2+, Claude Code, GitHub CLI 2.40+, git; building needs Rust with
`cargo-zigbuild`, Node 20+ and pnpm.

```bash
./build.sh
dist/packaging/install.sh
fleet-gui
```

The setup wizard walks you through your first persona. Full steps: [Install](docs/getting-started/install.md).

## Documentation

The [documentation](docs/index.md) covers:

- [Getting started](docs/getting-started/install.md): install and first run.
- [Guide](docs/guide/board.md): the board, products and releases, tasks, orchestrated tasks, personas.
- [Reference](docs/reference/configuration.md): configuration and architecture.
- [Contributing](docs/contributing/development.md): tests, UI internals, demo media.

## License

[Apache-2.0](LICENSE)
