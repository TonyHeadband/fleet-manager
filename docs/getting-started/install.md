# Install

Build fleet-manager from source and install the desktop app and the daemon into `~/.local`. Then go on to
[First run](first-run.md).

> **Status: v0.1, personal tool.** Linux only (x86_64 app; daemon on x86_64 and aarch64), Claude Code only,
> loopback only with no authentication. Expect breaking changes.

## Requirements

- Linux with glibc 2.28 or newer (Rocky/RHEL 8, Fedora, Nobara, …)
- tmux 3.2 or newer
- [Claude Code](https://docs.claude.com/en/docs/claude-code) (`claude`); you log in once per persona from the app
- [GitHub CLI](https://cli.github.com/) (`gh`) 2.40 or newer, for each persona's GitHub login
- git
- A terminal emulator (`$TERMINAL`, `x-terminal-emulator`, `gnome-terminal`, `konsole`, `xfce4-terminal` or
  `xterm`), where the app opens the `claude` and `gh` logins

## Build

Building for your own machine needs only Rust: `cargo build --release -p fleet-gui -p fleet-daemon`. zig and
`cargo-zigbuild` are only needed by `./build.sh`, to reproduce the glibc 2.28 Linux release build.

```bash
./build.sh
```

Runs the client, app and installer tests, and writes the binaries and
the installer to `dist/`. Needs Rust with `cargo-zigbuild`, a zig (`python3 -m venv ~/.cache/fleet-manager/zig-venv
&& ~/.cache/fleet-manager/zig-venv/bin/pip install ziglang`). Binaries are linked against
glibc 2.28 through zig, so one build runs on every distribution above it.

Each component also builds alone with `scripts/daemon.sh`, `scripts/gui.sh` or
`scripts/package.sh`; `build.sh` runs `cargo fmt --all --check` first, then the scripts in that order.
`.fleet/workflow.yml` lists the checks the Fleet app's pull-request gate runs on a branch, and which changed paths
trigger each. The web UI is not built: the daemon serves a placeholder page. It returns, as an option for
remote access, in a later release.

## Install

```bash
dist/packaging/install.sh            # or: --prefix DIR (default ~/.local)
```

Installs `fleet-daemon` and `fleet-gui` to `~/.local/bin`, plus a menu entry and icon. It writes no config and
touches nothing outside the prefix. Running it again upgrades in place.
