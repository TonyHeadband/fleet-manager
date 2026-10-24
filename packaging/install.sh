#!/usr/bin/env bash
# Installs fleet-manager for the current user: the daemon, the app and a menu launcher.
# Changes nothing outside the prefix (default ~/.local). It writes no config: the first launch of the app asks where
# projects live and writes ~/.config/fleet-manager/config.toml itself.
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
  echo "tmux was not found. fleet-manager needs tmux 3.2 or newer on PATH." >&2
  echo "On Rocky/RHEL 8 the packaged tmux is 2.7: build a newer one and put it in ~/.local/bin." >&2
  exit 1
fi
tmux_version="$(login 'tmux -V' || true)"
case "$tmux_version" in "tmux "*) ;; *) echo "could not read the tmux version (got: ${tmux_version:-nothing}) from $tmux_path" >&2; exit 1 ;; esac
read -r major minor <<<"$(sed -E 's/[^0-9]*([0-9]+)\.([0-9]+).*/\1 \2/' <<<"$tmux_version")"
if ! [[ "${major:-}" =~ ^[0-9]+$ && "${minor:-}" =~ ^[0-9]+$ ]] || [ "$major" -lt 3 ] || { [ "$major" -eq 3 ] && [ "$minor" -lt 2 ]; }; then
  echo "$tmux_version at $tmux_path is too old: fleet-manager needs tmux 3.2 or newer." >&2
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

# Quotes one argument for a .desktop Exec line. Inside double quotes the spec reserves " ` $ and \, escaped with a
# backslash; the value is also a string whose backslashes are escaped again, so each becomes two. % starts a field
# code. Done in bash, not sed, so | or & in a path cannot break it.
desktop_quote() {
  local s=$1
  s=${s//\\/\\\\\\\\}
  s=${s//\"/\\\\\"}
  s=${s//\`/\\\\\`}
  s=${s//\$/\\\\\$}
  s=${s//%/%%}
  printf '"%s"' "$s"
}

mkdir -p "$PREFIX/bin"
# Absolute, so the launcher's Exec line works from any directory.
PREFIX="$(cd "$PREFIX" && pwd)"
put "$DAEMON_SRC" "$PREFIX/bin/fleet-daemon"
if [ -f "$GUI_SRC" ]; then
  put "$GUI_SRC" "$PREFIX/bin/fleet-gui"
  put "$SRC/packaging/fleet-gui-launcher" "$PREFIX/bin/fleet-gui-launcher"
  mkdir -p "$PREFIX/share/applications" "$PREFIX/share/icons/hicolor/scalable/apps"
  cp "$SRC/packaging/fleet-manager.svg" "$PREFIX/share/icons/hicolor/scalable/apps/fleet-manager.svg"
  exec_line="Exec=$(desktop_quote "$PREFIX/bin/fleet-gui-launcher")"
  while IFS= read -r line; do
    [ "$line" = "Exec=@EXEC@" ] && line=$exec_line
    printf '%s\n' "$line"
  done < "$SRC/packaging/fleet-manager.desktop.in" > "$PREFIX/share/applications/fleet-manager.desktop"
  # The app was called fleet-board before; drop its launcher so the menu does not show both names.
  rm -f "$PREFIX/share/applications/fleet-board.desktop" "$PREFIX/share/icons/hicolor/scalable/apps/fleet-board.svg"
  command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$PREFIX/share/applications" >/dev/null 2>&1 || true
  command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$PREFIX/share/icons/hicolor" >/dev/null 2>&1 || true
else
  echo "note: no app for this machine type ($ARCH); only the daemon was installed."
fi

echo
echo "Installed to $PREFIX/bin."
echo
# An upgrade keeps its config, whose projects folder may not be the default; only a fresh install gets asked.
CONFIG="$HOME/.config/fleet-manager/config.toml"
root="$(sed -n -E 's/^[[:space:]]*projects_root[[:space:]]*=[[:space:]]*"([^"]*)".*/\1/p' "$CONFIG" 2>/dev/null | tail -1 || true)"
if [ -n "$root" ]; then
  case "$root" in /*|"~"|"~/"*) ;; *) root="" ;; esac
fi
if [ -n "$root" ]; then
  q="$(printf '%q' "$root")"
  [ "${root#\~/}" != "$root" ] && q="~/$(printf '%q' "${root#\~/}")"
  echo "One-time step, if not done yet: trust the projects folder from $CONFIG so agents start without a trust"
  echo "dialog:"
  echo "  mkdir -p $q && cd $q && claude"
else
  echo "One-time step: trust the projects folder so agents start without a trust dialog. The first launch asks where"
  echo "projects live (default ~/fleet-projects) and shows this command for the folder you choose:"
  echo "  mkdir -p ~/fleet-projects && cd ~/fleet-projects && claude"
fi
echo "Then choose \"Yes, I trust this folder\" (the second option: Down, Enter) and type /exit. The dialog starts on"
echo "\"No, exit\", so Enter alone or typing 1 quits instead."
echo
if [ -f "$GUI_SRC" ]; then
  echo "Launch it from the application menu (fleet-manager), or run: $PREFIX/bin/fleet-gui"
fi
case ":$(login 'echo "$PATH"'):" in
  *":$PREFIX/bin:"*) ;;
  *) echo "Note: $PREFIX/bin is not on your login PATH; add it to ~/.bash_profile." ;;
esac
