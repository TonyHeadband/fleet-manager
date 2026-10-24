#!/usr/bin/env bash
# Installs fleet-board for the current user: the daemon, the app, a config template and a menu launcher.
# Changes nothing outside the prefix (default ~/.local) and ~/.config/fleet-board, and never overwrites a config.
#
#   install.sh [--prefix DIR]
set -euo pipefail

PREFIX="$HOME/.local"
while [ $# -gt 0 ]; do
  case "$1" in
    --prefix) PREFIX="$2"; shift 2 ;;
    -h|--help) sed -n '2,6p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONFIG_DIR="$HOME/.config/fleet-board"
STATE_DIR="$HOME/.local/share/fleet-board"

case "$(uname -m)" in
  x86_64) ARCH=x86_64 ;;
  aarch64|arm64) ARCH=aarch64 ;;
  *) echo "unsupported machine type: $(uname -m)" >&2; exit 1 ;;
esac
DAEMON_SRC="$SRC/fleet-daemon-$ARCH-linux"
GUI_SRC="$SRC/fleet-gui-x86_64-linux"
[ -f "$DAEMON_SRC" ] || { echo "missing $DAEMON_SRC (run this from a build's dist directory)" >&2; exit 1; }

# Prerequisites, looked up the way the launcher will: through a login shell, so ~/.local/bin is on PATH.
# A profile may print a greeting or read stdin; only the last line (the answer) counts, and stdin is closed.
login() { bash -lc "$1" </dev/null 2>/dev/null | tail -n 1; }
tmux_path="$(login 'command -v tmux' || true)"
if [ -z "$tmux_path" ]; then
  echo "tmux was not found. fleet-board needs tmux 3.2 or newer on PATH." >&2
  echo "On Rocky/RHEL 8 the packaged tmux is 2.7: build a newer one and put it in ~/.local/bin." >&2
  exit 1
fi
tmux_version="$(login 'tmux -V' || true)"
case "$tmux_version" in "tmux "*) ;; *) echo "could not read the tmux version (got: ${tmux_version:-nothing}) from $tmux_path" >&2; exit 1 ;; esac
read -r major minor <<<"$(sed -E 's/[^0-9]*([0-9]+)\.([0-9]+).*/\1 \2/' <<<"$tmux_version")"
if ! [[ "${major:-}" =~ ^[0-9]+$ && "${minor:-}" =~ ^[0-9]+$ ]] || [ "$major" -lt 3 ] || { [ "$major" -eq 3 ] && [ "$minor" -lt 2 ]; }; then
  echo "$tmux_version at $tmux_path is too old: fleet-board needs tmux 3.2 or newer." >&2
  echo "Install a newer tmux and put it first on PATH (for example in ~/.local/bin), then run this again." >&2
  exit 1
fi
claude_path="$(login 'command -v claude' || true)"
if [ -z "$claude_path" ]; then
  echo "claude was not found on PATH. Install Claude Code and log in, then run this again." >&2
  exit 1
fi
echo "tmux   $tmux_path ($tmux_version)"
echo "claude $claude_path"

# Copy then move, so replacing a running binary during an upgrade does not fail with "text file busy".
put() { # src dest
  local tmp
  tmp="$(mktemp "$2.XXXXXX")"
  cp "$1" "$tmp" && chmod 755 "$tmp" && mv -f "$tmp" "$2"
}

mkdir -p "$PREFIX/bin" "$CONFIG_DIR" "$STATE_DIR/worktrees"
put "$DAEMON_SRC" "$PREFIX/bin/fleet-daemon"
if [ -f "$GUI_SRC" ]; then
  put "$GUI_SRC" "$PREFIX/bin/fleet-gui"
  mkdir -p "$PREFIX/share/applications" "$PREFIX/share/icons/hicolor/scalable/apps"
  cp "$SRC/packaging/fleet-board.svg" "$PREFIX/share/icons/hicolor/scalable/apps/fleet-board.svg"
  sed "s|@BIN@|$PREFIX/bin/fleet-gui|g" "$SRC/packaging/fleet-board.desktop.in" > "$PREFIX/share/applications/fleet-board.desktop"
  command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$PREFIX/share/applications" >/dev/null 2>&1 || true
  command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$PREFIX/share/icons/hicolor" >/dev/null 2>&1 || true
else
  echo "note: no app for this machine type ($ARCH); only the daemon was installed."
fi

if [ ! -e "$CONFIG_DIR/config.toml" ]; then
  cp "$SRC/packaging/config.example.toml" "$CONFIG_DIR/config.toml"
  echo "wrote $CONFIG_DIR/config.toml: add your repos there"
else
  echo "kept your existing $CONFIG_DIR/config.toml"
fi

echo
echo "Installed to $PREFIX/bin."
echo
echo "One-time step: trust the worktrees folder so agents start without a trust dialog:"
echo "  cd $STATE_DIR/worktrees && claude     # accept \"trust this folder\", then type /exit"
echo
if [ -f "$GUI_SRC" ]; then
  echo "Launch it from the application menu (fleet-board), or run: $PREFIX/bin/fleet-gui"
fi
case ":$(login 'echo "$PATH"'):" in
  *":$PREFIX/bin:"*) ;;
  *) echo "Note: $PREFIX/bin is not on your login PATH; add it to ~/.bash_profile." ;;
esac
