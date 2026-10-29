# First run

From a fresh install to a first agent on the board: let the wizard check your tools, set up a persona, create a
product, start a task. It assumes Fleet Manager is [installed](install.md).

## Set up

1. Run `fleet-gui` (`fleet-gui.exe` on Windows), or launch **Fleet Manager** from its shortcut.
2. While a progress bar runs under the logo, the app starts its background service (the daemon) and asks it what
   this machine has. Everything the app shows about your tools comes from the daemon.
3. The setup wizard runs whenever no persona exists:
   1. **Where Fleet Manager lives**: where tasks live (default `fleet-tasks` in your home folder) and where
      personas live (default in the data folder, see [Install](install.md#where-your-data-lives)). This writes
      `config.toml`.
   2. **What Fleet Manager needs**: a row per tool (Git, Git Bash on Windows, Claude Code, GitHub CLI) with what
      the daemon found. A missing or too-old tool shows its install command: **Install** runs it in a terminal
      window (Anthropic's installer for Claude Code; winget on Windows or your package manager on Linux for the
      others, which asks for your password), **Copy** copies it,
      and the link opens the tool's download page. **Check again** asks the daemon to look again, with a fresh
      `PATH`, so a tool you just installed is found without restarting. **Continue without it** goes on anyway;
      the last step lists what is still missing.
   3. **Your first persona**: a name, the commit name and email, the GitHub host, and how to log in to it
      (**gh login** or **Personal access token**).
   4. **Log in**: **Claude** opens a terminal (Git Bash's on Windows) running `claude` in the tasks folder with the
      persona's own config folder. Trust the folder first (choose **"Yes, I trust this folder"**: Down, then
      Enter; the dialog starts on "No, exit"), then type `/login`. **GitHub** opens `gh auth login` for the
      persona's host, or takes a personal access token that the app pipes to `gh` once. Either way `gh` keeps the
      token in the persona's folder (`--insecure-storage`), not the system keyring. Each line shows the account
      (green) once the CLI reports a login. Tick **Add a Start menu shortcut** (Windows) or **Add an app menu
      shortcut** (Linux) if you want one; **Finish** is enabled when both logins are.
4. **New product**: Management → Products → **+ New product**. Give it a name and add its repos (a local path, a
   URL, or both).
5. **New task**: give it a goal, pick the persona (the active one by default) and, optionally, repos of the
   product. The card appears and the agent starts.

## Later

- **Settings → Extras** shows what the daemon found for each required tool, with **Check again**. Installing is
  left to the wizard.
- **Relaunch setup wizard** (at the bottom of Settings → General, and in Extras) runs the tool check and the
  folders again. Folder changes are
  saved at once and take effect when the daemon restarts; the wizard offers **Restart now**.

## More personas

Management → Personas → **+ New persona** runs the persona and login steps. What a persona holds, how it keeps
logins apart and what happens when a login breaks: [Personas](../guide/personas.md).

## Permission mode

Agents only reach Needs you when Claude Code asks before running tools. If your own Claude settings default to a
mode that never asks, set `claude_command = ["claude", "--permission-mode", "default"]` in the
[config](../reference/configuration.md).

## Next

- [The board](../guide/board.md): what the columns mean and what you can do from a card.
- [Tasks and repos](../guide/tasks-and-repos.md): how branches and merging work.
