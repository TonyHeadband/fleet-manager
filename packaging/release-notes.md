<!-- GitHub release notes keep every line break: one line per paragraph or bullet, no hard wrapping. -->
## Download

| System | File |
|---|---|
| Windows 10 (1809+) or 11, x64 | `fleet-manager-v…-windows-x64.zip` |
| Linux x86_64, glibc 2.28+ (Rocky/RHEL 8 and newer) | `fleet-manager-v…-linux-x64.tar.gz` |

Unpack it anywhere and run `fleet-gui` (`fleet-gui.exe` on Windows). The setup wizard checks for Git, Claude Code and GitHub CLI and offers to install what is missing. Windows may say "Windows protected your PC" because the app is not signed: click **More info**, then **Run anyway**. Full steps: [Install](https://tonyheadband.github.io/fleet-manager/getting-started/install/).

To update, close Fleet Manager with **Stop the daemon too** and unpack the new archive over the old folder. `SHA256SUMS` lists each file's checksum.

**Feedback is very welcome:** [open an issue](https://github.com/TonyHeadband/fleet-manager/issues/new/choose) for a bug or an idea.
