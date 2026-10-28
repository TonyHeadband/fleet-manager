#!/usr/bin/env bash
# Installs fleet-manager for the current user: the daemon, fleet-attach, the app and a menu launcher.
# Changes nothing outside the prefix (default ~/.local). It writes no config: the first launch of the app asks where
# tasks live and writes ~/.config/fleet-manager/config.toml itself.
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
# The daemon names the fleet-attach beside its own executable in the command that takes over an agent's terminal.
ATTACH_SRC="$SRC/fleet-attach-$ARCH-linux"
GUI_SRC="$SRC/fleet-gui-x86_64-linux"
for f in "$DAEMON_SRC" "$ATTACH_SRC"; do
  [ -f "$f" ] || { echo "missing $f (run this from a build's dist directory)" >&2; exit 1; }
done

# Prerequisites, looked up the way the launcher will: through a login shell, so ~/.local/bin is on PATH.
# A profile may print a greeting or read stdin; only the last line (the answer) counts, and stdin is closed.
login() { bash -lc "$1" </dev/null 2>/dev/null | tail -n 1; }
CONFIG="$HOME/.config/fleet-manager/config.toml"
claude_path="$(login 'command -v claude' || true)"
if [ -z "$claude_path" ]; then
  echo "claude was not found on PATH. Install Claude Code, then run this again; the app logs it in per persona." >&2
  exit 1
fi
# Each persona's GitHub login is gh's own, kept in the persona's folder by `gh auth login --insecure-storage`,
# which gh has had since 2.40. The gh an existing config names (gh_command) is the one checked.
gh_cmd="$(sed -n -E 's/^[[:space:]]*gh_command[[:space:]]*=[[:space:]]*"([^"]*)".*/\1/p' "$CONFIG" 2>/dev/null | tail -1 || true)"
case "$gh_cmd" in "~/"*) gh_cmd="$HOME/${gh_cmd#\~/}" ;; esac
gh_cmd="${gh_cmd:-gh}"
gh_path="$(login "command -v $(printf '%q' "$gh_cmd")" || true)"
if [ -z "$gh_path" ]; then
  echo "$gh_cmd was not found. fleet-manager needs the GitHub CLI (gh) 2.40 or newer on PATH (or named by gh_command in $CONFIG)." >&2
  exit 1
fi
gh_version="$(bash -lc "$(printf '%q' "$gh_path") --version" </dev/null 2>/dev/null | grep -m1 '^gh version ' || true)"
read -r gh_major gh_minor <<<"$(sed -E 's/^gh version ([0-9]+)\.([0-9]+).*/\1 \2/' <<<"$gh_version")"
if ! [[ "${gh_major:-}" =~ ^[0-9]+$ && "${gh_minor:-}" =~ ^[0-9]+$ ]]; then
  echo "could not read the gh version (got: ${gh_version:-nothing}) from $gh_path" >&2
  exit 1
fi
if [ "$gh_major" -lt 2 ] || { [ "$gh_major" -eq 2 ] && [ "$gh_minor" -lt 40 ]; }; then
  echo "$gh_version at $gh_path is too old: fleet-manager needs gh 2.40 or newer." >&2
  echo "Install a newer gh and put it first on PATH (for example in ~/.local/bin), then run this again." >&2
  exit 1
fi
echo "claude $claude_path"
echo "gh     $gh_path (${gh_version#gh version })"

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
put "$ATTACH_SRC" "$PREFIX/bin/fleet-attach"
if [ -f "$GUI_SRC" ]; then
  put "$GUI_SRC" "$PREFIX/bin/fleet-gui"
  put "$SRC/packaging/fleet-gui-launcher" "$PREFIX/bin/fleet-gui-launcher"
  mkdir -p "$PREFIX/share/applications" "$PREFIX/share/icons/hicolor/scalable/apps"
  cp "$SRC/packaging/fleet-manager.svg" "$PREFIX/share/icons/hicolor/scalable/apps/fleet-manager.svg"
  # Below 64 px the full icon's hairlines blur; these sizes get a simplified, solid version.
  for n in 16 32 48; do
    mkdir -p "$PREFIX/share/icons/hicolor/${n}x${n}/apps"
    cp "$SRC/packaging/icons/$n/fleet-manager.png" "$PREFIX/share/icons/hicolor/${n}x${n}/apps/fleet-manager.png"
  done
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
echo "The first launch sets up a persona; its Claude login trusts the tasks folder, so agents start without a"
echo "trust dialog."
echo
if [ -f "$GUI_SRC" ]; then
  echo "Launch it from the application menu (fleet-manager), or run: $PREFIX/bin/fleet-gui"
fi
case ":$(login 'echo "$PATH"'):" in
  *":$PREFIX/bin:"*) ;;
  *) echo "Note: $PREFIX/bin is not on your login PATH; add it to ~/.bash_profile." ;;
esac
