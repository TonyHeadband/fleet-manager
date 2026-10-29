<p align="center">
  <img src="docs/assets/logo.svg" alt="" width="96">
</p>

<h1 align="center">Fleet Manager</h1>

<p align="center">
  <em>Your agents, on a board, not behind a wall of text.</em><br>
  A desktop command room for a fleet of coding agents.
</p>

<p align="center">
  <a href="https://tonyheadband.github.io/fleet-manager/"><img src="https://img.shields.io/badge/docs-tonyheadband.github.io-d97757" alt="Documentation"></a>
  <img src="https://img.shields.io/badge/platform-Windows%20%7C%20Linux-informational" alt="Windows | Linux">
  <img src="https://img.shields.io/badge/agents-Claude%20Code-d97757" alt="Claude Code">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-green" alt="Apache-2.0"></a>
</p>

![Fleet Manager: agents moving across the board](docs/assets/demo.gif)

- **Fly by hand.** Answer, pause and review each agent from its card, with its live terminal and diff one click
  away.
- **Autopilot.** An orchestrator agrees the goal with you, breaks it down and runs specialists under a policy you
  set.
- **Specialists.** Narrow agents with one job each, whose limits Fleet Manager enforces, not only their prompt.

With a few agents running in their own terminals, you cannot see who is working, who is stuck and who is waiting
on you. Here every agent, subagents included, is a card whose status comes live from Claude Code hooks, so you
answer the card that needs you. More in [what fleet management means here](https://tonyheadband.github.io/fleet-manager/concept/).

Today you fly by hand, and orchestrated tasks run with you approving each step. Autopilot and enforced limits are
next: see [ROADMAP.md](ROADMAP.md).

## Quick start

1. Download the archive for your system from the [latest release](https://github.com/TonyHeadband/fleet-manager/releases/latest):
   `fleet-manager-v<version>-windows-x64.zip` (Windows 10 1809+ or 11) or
   `fleet-manager-v<version>-linux-x64.tar.gz` (x86_64, glibc 2.28+: Rocky/RHEL 8 and newer).
2. Unpack it anywhere and run `fleet-gui` (`fleet-gui.exe` on Windows).
3. The setup wizard checks for Git (with Git Bash on Windows), Claude Code and GitHub CLI 2.40+, offers to install
   any that are missing, then walks you through your first persona.

Nothing is installed system-wide; to update, unpack the new version over the old folder. Full steps:
[Install](https://tonyheadband.github.io/fleet-manager/getting-started/install/). To build from source, see
[Development](https://tonyheadband.github.io/fleet-manager/contributing/development/).

## Documentation

| Section | What it covers |
|---|---|
| [**Getting started**](https://tonyheadband.github.io/fleet-manager/getting-started/install/) | Install, then your first persona and task. |
| [**Guide**](https://tonyheadband.github.io/fleet-manager/guide/board/) | The board, products and releases, tasks, orchestrated tasks, personas, management. |
| [**Reference**](https://tonyheadband.github.io/fleet-manager/reference/configuration/) | Configuration and architecture. |
| [**Contributing**](https://tonyheadband.github.io/fleet-manager/contributing/development/) | Tests, UI internals, demo media. |

## Contributing

Fleet Manager is under continuous development, so unsolicited pull requests are not accepted yet; contributions
will open once it reaches a stable stage. If you would like to help now, open an issue asking to contribute and
say what you have in mind. Bug reports and feature requests are welcome as issues.

## License

[Apache-2.0](LICENSE)
