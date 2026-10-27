# First run

From a fresh install to a first agent on the board: set up a persona, create a product, start a task. It assumes
fleet-manager is [installed](install.md).

## Set up

1. Launch **fleet-manager** from the application menu, or run `fleet-gui`.
2. The setup wizard runs whenever no persona exists (also after upgrading an install from before personas):
   1. **Folders**: where tasks live (default `~/fleet-tasks`) and where personas live (default
      `~/.local/share/fleet-manager/personas`). This writes `~/.config/fleet-manager/config.toml`.
   2. **Your first persona**: a name, the commit name and email, the GitHub host, and how to log in to it
      (**gh login** or **Personal access token**).
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

## More personas

Management → Personas → **+ New persona** runs steps 2 and 3. What a persona holds, how it keeps logins apart
and what happens when a login breaks: [Personas](../guide/personas.md).

## Permission mode

Agents only reach Needs you when Claude Code asks before running tools. If your own Claude settings default to a
mode that never asks, set `claude_command = ["claude", "--permission-mode", "default"]` in the
[config](../reference/configuration.md).

## Next

- [The board](../guide/board.md): what the columns mean and what you can do from a card.
- [Tasks and repos](../guide/tasks-and-repos.md): how branches and merging work.
