#!/usr/bin/env bash
# Tests for install.sh: each case runs it in a fresh temporary HOME with fake claude and gh.
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
  cp -r "$HERE"/icons "$T/dist/packaging/"
  arch="$(uname -m)"
  for b in "fleet-daemon-$arch-linux" "fleet-attach-$arch-linux" "fleet-gui-x86_64-linux"; do
    printf '#!/bin/sh\necho "%s"\n' "$b" > "$T/dist/$b"
    chmod 755 "$T/dist/$b"
  done
  printf '#!/bin/sh\necho claude\n' > "$T/fakebin/claude"
  printf '#!/bin/sh\necho "gh version %s (2024-01-01)"\necho "https://github.com/cli/cli/releases/tag/v%s"\n' "${GH_VERSION:-2.62.0}" "${GH_VERSION:-2.62.0}" > "$T/fakebin/gh"
  chmod 755 "$T/fakebin/claude" "$T/fakebin/gh"
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

# 1b: beside the daemon, which names it in attach commands
if [ -x "$T/home/.local/bin/fleet-attach" ] && [ "$("$T/home/.local/bin/fleet-attach")" = "fleet-attach-$(uname -m)-linux" ]; then
  pass installs_fleet_attach_beside_the_daemon
else fail installs_fleet_attach_beside_the_daemon "$(ls -l "$T/home/.local/bin" 2>&1)"; fi

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

# 6
fixture; rm "$T/fakebin/claude"; run_install
if [ "$(status)" != 0 ] && grep -qi "claude" "$T/out" && [ ! -e "$T/home/.local/bin/fleet-gui" ]; then pass fails_with_a_message_when_claude_is_missing
else fail fails_with_a_message_when_claude_is_missing "status $(status): $(cat "$T/out")"; fi
# 6b: every persona's GitHub login is gh's, stored with --insecure-storage (gh 2.40)
# The machine may have a gh in /usr/bin, so "missing" is a gh_command that names nothing.
fixture; mkdir -p "$T/home/.config/fleet-manager"
printf 'gh_command = "%s"\n' "$T/nonexistent/gh" > "$T/home/.config/fleet-manager/config.toml"
run_install
if [ "$(status)" != 0 ] && grep -q "GitHub CLI (gh) 2.40" "$T/out" && [ ! -e "$T/home/.local/bin/fleet-daemon" ]; then pass fails_with_a_message_when_gh_is_missing
else fail fails_with_a_message_when_gh_is_missing "status $(status): $(cat "$T/out")"; fi
GH_VERSION=2.39.2 fixture; run_install
if [ "$(status)" != 0 ] && grep -q "2.39.2" "$T/out" && grep -q "2.40" "$T/out" && [ ! -e "$T/home/.local/bin/fleet-daemon" ]; then pass fails_when_gh_is_older_than_2_40
else fail fails_when_gh_is_older_than_2_40 "status $(status): $(cat "$T/out")"; fi
GH_VERSION=2.40.0 fixture; run_install
if [ "$(status)" = 0 ] && grep -q "gh .*(2.40.0 (2024-01-01))" "$T/out"; then pass accepts_gh_2_40_and_prints_its_version
else fail accepts_gh_2_40_and_prints_its_version "status $(status): $(cat "$T/out")"; fi
GH_VERSION=2.39.0 fixture; mkdir -p "$T/home/.config/fleet-manager" "$T/opt"
printf '#!/bin/sh\necho "gh version 2.45.0 (2024-03-04)"\n' > "$T/opt/gh"; chmod 755 "$T/opt/gh"
printf 'gh_command = "%s"\n' "$T/opt/gh" > "$T/home/.config/fleet-manager/config.toml"
run_install
if [ "$(status)" = 0 ] && grep -q "$T/opt/gh (2.45.0" "$T/out"; then pass the_gh_check_follows_gh_command
else fail the_gh_check_follows_gh_command "status $(status): $(cat "$T/out")"; fi

# 7
fixture; run_install
# Trust is per persona now: the old step (run claude in the folder) would only trust it for ~/.claude.
grep -q "first launch sets up a persona" "$T/out" && grep -q "Claude login trusts the tasks folder" "$T/out" \
  && ! grep -q "&& claude" "$T/out" && ! grep -qi "trust this folder" "$T/out" && ! grep -q "worktrees" "$T/out" \
  && pass says_the_persona_login_trusts_the_folder || fail says_the_persona_login_trusts_the_folder "$(cat "$T/out")"
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
fixture; printf 'echo "Welcome to the workstation"\necho "gh version 1.0 is mentioned here"\n' > "$T/home/.bash_profile"; run_install
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

# 10b: an upgrade is not told to trust its tasks folder by hand either; its first launch sets up a persona
fixture; mkdir -p "$T/home/.config/fleet-manager"
printf 'listen = "127.0.0.1:7878"\ntasks_root = "~/work/my agents"\n' > "$T/home/.config/fleet-manager/config.toml"
run_install
if [ "$(status)" = 0 ] && grep -q "first launch sets up a persona" "$T/out" && ! grep -q "my agents" "$T/out" && ! grep -q "&& claude" "$T/out"; then
  pass an_upgrade_gets_no_manual_trust_step
else fail an_upgrade_gets_no_manual_trust_step "$(cat "$T/out")"; fi

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

# 13: small sizes get the simplified icon, beside the scalable one
fixture; run_install
icons="$T/home/.local/share/icons/hicolor"
missing=""
for n in 16 32 48; do [ -f "$icons/${n}x${n}/apps/fleet-manager.png" ] || missing="$missing ${n}x${n}"; done
[ "$(status)" = 0 ] && [ -z "$missing" ] && [ -f "$icons/scalable/apps/fleet-manager.svg" ] \
  && pass installs_small_icons || fail installs_small_icons "missing:$missing; status $(status): $(cat "$T/out")"

echo
[ "$fails" = 0 ] && echo "all install tests passed" || echo "$fails install test(s) failed"
exit "$fails"
