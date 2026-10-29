# Install

Fleet Manager ships as one archive per system. Unpack it, run it, and the setup wizard does the rest. Then go on
to [First run](first-run.md).

> **Status: v0.1, personal tool.** Windows and Linux, Claude Code only, loopback only with no authentication.
> Expect breaking changes.

## Requirements

- **Windows** 10 (1809 or newer) or 11, x64; or **Linux** x86_64 with glibc 2.28 or newer (Rocky/RHEL 8,
  Fedora, Nobara, Ubuntu 20.04+, …)
- [Git](https://git-scm.com/downloads); on Windows, Git for Windows, whose Git Bash runs the agents' shells and the
  login terminals
- [Claude Code](https://docs.claude.com/en/docs/claude-code/setup) (`claude`); you log in once per persona from the
  app
- [GitHub CLI](https://cli.github.com/) (`gh`) 2.40 or newer, for each persona's GitHub login
- On Linux, a terminal emulator (`$TERMINAL`, `x-terminal-emulator`, `gnome-terminal`, `konsole`,
  `xfce4-terminal` or `xterm`), where the app opens installs and logins

You don't need these before you start: the setup wizard checks for them and offers to install any that are
missing.

## Download and run

1. Download the archive for your system from
   [Releases](https://github.com/TonyHeadband/fleet-manager/releases):
    - Windows: `fleet-manager-v<version>-windows-x64.zip`
    - Linux: `fleet-manager-v<version>-linux-x64.tar.gz`
2. Unpack it anywhere you like, for example your home folder. It holds one folder, `fleet-manager`.
3. Run `fleet-gui.exe` (Windows) or `fleet-gui` (Linux) from that folder.

On Windows, SmartScreen may say "Windows protected your PC" because the app is not signed: click **More info**,
then **Run anyway**.

Nothing is installed system-wide. The wizard's last step offers a **Fleet Manager** shortcut in the Start menu
(Windows) or the application menu (Linux); Settings → General → Shortcuts adds, updates or removes Start menu and
desktop shortcuts later, for example after you move the folder.

## Where your data lives

The app folder holds only the programs. Settings, tasks and personas live in your user folders, so they survive
updates:

| | Windows | Linux |
|---|---|---|
| Config (`config.toml`, `gui.toml`) | `%USERPROFILE%\AppData\Roaming\fleet-manager` | `~/.config/fleet-manager` |
| Data (database, logs, personas) | `%USERPROFILE%\AppData\Local\fleet-manager` | `~/.local/share/fleet-manager` |
| Tasks (default, chosen in the wizard) | `%USERPROFILE%\fleet-tasks` | `~/fleet-tasks` |

## Update

When a new version is out, the app shows a notice when it starts (and a line in Settings); **Download** opens the
release page. Tick **Skip this version** to stop the notice for that version. To update:

1. Close Fleet Manager and choose **Stop the daemon too** in the close dialog. Running agents stop and are resumed
   when it starts again.
2. Unpack the new archive over the old `fleet-manager` folder.
3. Start it again.

If a background service (the daemon) from the old version is still running, the app says so and offers to restart
it.

## Uninstall

Delete the `fleet-manager` folder and the shortcut. To remove your data too, delete the folders in the table above.

## Build from source

See [Development](../contributing/development.md).
