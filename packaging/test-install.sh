#!/usr/bin/env bash
# Tests for install.sh: each case runs it in a fresh temporary HOME with fake tmux and claude.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
fails=0
pass() { echo "ok   $1"; }
fail() { echo "FAIL $1: $2"; fails=$((fails + 1)); }

# A fake dist directory (packaging files plus stand-in binaries) and a fake bin directory.
fixture() {
  T="$(mktemp -d)"
  mkdir -p "$T/dist/packaging" "$T/fakebin" "$T/home"
  cp "$HERE"/install.sh "$HERE"/fleet-gui-launcher "$HERE"/fleet-manager.desktop.in "$HERE"/fleet-manager.svg "$HERE"/config.example.toml "$T/dist/packaging/"
  arch="$(uname -m)"
  for b in "fleet-daemon-$arch-linux" "fleet-gui-x86_64-linux"; do
    printf '#!/bin/sh\necho "%s"\n' "$b" > "$T/dist/$b"
    chmod 755 "$T/dist/$b"
  done
  printf '#!/bin/sh\necho "tmux %s"\n' "${TMUX_VERSION:-3.4}" > "$T/fakebin/tmux"
  printf '#!/bin/sh\necho claude\n' > "$T/fakebin/claude"
  chmod 755 "$T/fakebin/tmux" "$T/fakebin/claude"
}

run_install() { # extra args pass through; output in $T/out, status in $T/status
  ( cd "$T" && HOME="$T/home" PATH="$T/fakebin:/usr/bin:/bin" bash "$T/dist/packaging/install.sh" "$@" > "$T/out" 2>&1; echo $? > "$T/status" )
}
status() { cat "$T/status"; }

# 1
fixture; run_install
if [ "$(status)" = 0 ] && [ -x "$T/home/.local/bin/fleet-daemon" ] && [ -x "$T/home/.local/bin/fleet-gui" ] \
  && [ -f "$T/home/.local/share/applications/fleet-manager.desktop" ] && [ -f "$T/home/.local/share/icons/hicolor/scalable/apps/fleet-manager.svg" ]; then
  pass installs_both_binaries_and_the_launcher_and_icon
else fail installs_both_binaries_and_the_launcher_and_icon "status $(status): $(cat "$T/out")"; fi

# 2
d="$T/home/.local/share/applications/fleet-manager.desktop"
if grep -qx "Exec=\"$T/home/.local/bin/fleet-gui-launcher\"" "$d" && ! grep -q "@EXEC@" "$d" && grep -q "^Icon=fleet-manager" "$d" && grep -q "^StartupWMClass=fleet-manager" "$d" \
  && grep -q "bash -lc" "$T/home/.local/bin/fleet-gui-launcher"; then
  pass launcher_exec_runs_the_login_shell_launcher
else fail launcher_exec_runs_the_login_shell_launcher "$(cat "$d" 2>&1)"; fi

# 3: the first launch of the app writes the config, so the installer must not
if [ ! -e "$T/home/.config/fleet-manager/config.toml" ]; then pass writes_no_config
else fail writes_no_config "config.toml exists"; fi
[ ! -d "$T/home/.local/share/fleet-manager/worktrees" ] && pass creates_no_worktrees_folder || fail creates_no_worktrees_folder "present"

# 4
fixture; mkdir -p "$T/home/.config/fleet-manager"; echo "# mine, do not touch" > "$T/home/.config/fleet-manager/config.toml"
run_install
if [ "$(status)" = 0 ] && [ "$(cat "$T/home/.config/fleet-manager/config.toml")" = "# mine, do not touch" ]; then pass never_overwrites_an_existing_config
else fail never_overwrites_an_existing_config "$(cat "$T/home/.config/fleet-manager/config.toml")"; fi

# 5
TMUX_VERSION=2.7 fixture; run_install
if [ "$(status)" != 0 ] && grep -q "2.7" "$T/out" && grep -q "3.2" "$T/out" && [ ! -e "$T/home/.local/bin/fleet-daemon" ]; then pass fails_with_the_fix_when_tmux_is_older_than_3_2
else fail fails_with_the_fix_when_tmux_is_older_than_3_2 "status $(status): $(cat "$T/out")"; fi
TMUX_VERSION=3.3a fixture; run_install
[ "$(status)" = 0 ] && pass accepts_a_lettered_tmux_version || fail accepts_a_lettered_tmux_version "$(cat "$T/out")"
TMUX_VERSION=next-3.5 fixture; run_install
[ "$(status)" = 0 ] && pass accepts_a_next_tmux_build || fail accepts_a_next_tmux_build "$(cat "$T/out")"

# 6
fixture; rm "$T/fakebin/claude"; run_install
if [ "$(status)" != 0 ] && grep -qi "claude" "$T/out" && [ ! -e "$T/home/.local/bin/fleet-gui" ]; then pass fails_with_a_message_when_claude_is_missing
else fail fails_with_a_message_when_claude_is_missing "status $(status): $(cat "$T/out")"; fi
fixture; rm "$T/fakebin/tmux"; run_install
[ "$(status)" != 0 ] && grep -qi "tmux" "$T/out" && pass fails_when_tmux_is_missing || fail fails_when_tmux_is_missing "$(cat "$T/out")"

# 7
fixture; run_install
grep -qi "trust this folder" "$T/out" && grep -q "second option" "$T/out" && ! grep -qi "accept" "$T/out" && grep -q "fleet-projects" "$T/out" && ! grep -q "worktrees" "$T/out" && pass prints_the_one_time_trust_step || fail prints_the_one_time_trust_step "$(cat "$T/out")"
grep -q "$T/home/.local/bin/fleet-gui" "$T/out" || grep -qi "launch" "$T/out" && pass says_how_to_start_it || fail says_how_to_start_it "$(cat "$T/out")"

# 8: only files the installer itself creates count. A login shell may write dotfiles of its own (system profile
# scripts do on some machines), so take the baseline after one login shell has run in this HOME.
fixture
HOME="$T/home" PATH="$T/fakebin:/usr/bin:/bin" bash -lc true >/dev/null 2>&1
( cd "$T/home" && find . -type f | sort > "$T/files.before" )
run_install
( cd "$T/home" && find . -type f | sort > "$T/files.after" )
new="$(comm -13 "$T/files.before" "$T/files.after")"
stray="$(echo "$new" | grep -v -E '^\./\.(local|config)/' | grep -v '^$' || true)"
elsewhere="$(cd "$T" && find . -path ./home -prune -o -type f -newer "$T/files.before" -print | grep -v -E '^\./(out|status|files\.(before|after))$' || true)"
if [ -z "$stray" ] && [ -z "$elsewhere" ] && [ -n "$new" ]; then pass touches_nothing_outside_home_local_and_config
else fail touches_nothing_outside_home_local_and_config "stray: $stray elsewhere: $elsewhere"; fi

# 8b: a site login profile that prints (a greeting, a motd) must not corrupt the version check
fixture; printf 'echo "Welcome to the workstation"\necho "tmux 1.0 is mentioned here"\n' > "$T/home/.bash_profile"; run_install
if [ "$(status)" = 0 ] && [ -x "$T/home/.local/bin/fleet-daemon" ]; then pass a_login_profile_that_prints_does_not_break_the_prerequisite_check
else fail a_login_profile_that_prints_does_not_break_the_prerequisite_check "status $(status): $(cat "$T/out")"; fi

# 9
fixture; run_install; run_install
[ "$(status)" = 0 ] && pass a_second_run_upgrades_in_place || fail a_second_run_upgrades_in_place "$(cat "$T/out")"

# 9b
fixture; mkdir -p "$T/home/.local/share/applications" "$T/home/.local/share/icons/hicolor/scalable/apps"
touch "$T/home/.local/share/applications/fleet-board.desktop" "$T/home/.local/share/icons/hicolor/scalable/apps/fleet-board.svg"
run_install
[ "$(status)" = 0 ] && [ ! -e "$T/home/.local/share/applications/fleet-board.desktop" ] \
  && [ ! -e "$T/home/.local/share/icons/hicolor/scalable/apps/fleet-board.svg" ] \
  && pass removes_the_old_fleet_board_launcher || fail removes_the_old_fleet_board_launcher "$(cat "$T/out")"

# 10
fixture; run_install --prefix "$T/prefix"
[ "$(status)" = 0 ] && [ -x "$T/prefix/bin/fleet-daemon" ] && [ -x "$T/prefix/bin/fleet-gui" ] && pass honours_prefix || fail honours_prefix "$(cat "$T/out")"

# 10a: an upgrade whose config names its own tmux is checked against that one, not the old tmux on PATH
TMUX_VERSION=2.7 fixture; mkdir -p "$T/home/.config/fleet-manager" "$T/opt"
printf '#!/bin/sh\necho "tmux 3.5"\n' > "$T/opt/tmux"; chmod 755 "$T/opt/tmux"
printf 'tmux_command = "%s"\n' "$T/opt/tmux" > "$T/home/.config/fleet-manager/config.toml"
run_install
if [ "$(status)" = 0 ] && grep -q "$T/opt/tmux (tmux 3.5)" "$T/out"; then pass the_tmux_check_follows_tmux_command
else fail the_tmux_check_follows_tmux_command "status $(status): $(cat "$T/out")"; fi

# 10b: an upgrade's config names its own projects folder; the trust step must use it
fixture; mkdir -p "$T/home/.config/fleet-manager"
printf 'listen = "127.0.0.1:7878"\nprojects_root = "~/work/my agents"\n' > "$T/home/.config/fleet-manager/config.toml"
run_install
if [ "$(status)" = 0 ] && grep -qF 'mkdir -p ~/work/my\ agents && cd ~/work/my\ agents && claude' "$T/out" && ! grep -q "fleet-projects" "$T/out"; then
  pass the_trust_step_follows_an_existing_projects_root
else fail the_trust_step_follows_an_existing_projects_root "$(cat "$T/out")"; fi

# 11: the Exec line must survive a HOME with a space and a prefix with characters sed or a shell would mangle
# The argument an Exec="…" line holds, undoing the desktop-entry escapes (\\\\ is one backslash, \\X is X, %% is %).
desktop_unquote() {
  local s=${1#Exec=\"}
  s=${s%\"}
  s=${s//%%/%}
  s=${s//\\\\\\\\/$'\x01'}
  s=${s//\\\\/}
  s=${s//$'\x01'/\\}
  printf '%s' "$s"
}
fixture; mkdir -p "$T/my home"
odd="$T/my home/a|b&c\$d"
( cd "$T" && HOME="$T/my home" PATH="$T/fakebin:/usr/bin:/bin" bash "$T/dist/packaging/install.sh" --prefix "$odd" > "$T/out" 2>&1; echo $? > "$T/status" )
line="$(grep '^Exec=' "$odd/share/applications/fleet-manager.desktop" 2>/dev/null)"
if [ "$(status)" = 0 ] && [ "$(desktop_unquote "$line")" = "$odd/bin/fleet-gui-launcher" ] \
  && [ "$(HOME="$T/my home" PATH="/usr/bin:/bin" "$(desktop_unquote "$line")")" = "fleet-gui-x86_64-linux" ]; then
  pass the_launcher_survives_spaces_and_shell_characters
else fail the_launcher_survives_spaces_and_shell_characters "status $(status), exec: $line, out: $(cat "$T/out")"; fi

# 12
fixture; run_install --prefix rel/prefix
line="$(grep '^Exec=' "$T/rel/prefix/share/applications/fleet-manager.desktop" 2>/dev/null)"
[ "$(status)" = 0 ] && [ "$(desktop_unquote "$line")" = "$T/rel/prefix/bin/fleet-gui-launcher" ] \
  && pass a_relative_prefix_gives_an_absolute_exec || fail a_relative_prefix_gives_an_absolute_exec "exec: $line, out: $(cat "$T/out")"

echo
[ "$fails" = 0 ] && echo "all install tests passed" || echo "$fails install test(s) failed"
exit "$fails"
