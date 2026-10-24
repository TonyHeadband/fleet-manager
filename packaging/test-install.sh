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
  cp "$HERE"/install.sh "$HERE"/fleet-board.desktop.in "$HERE"/fleet-board.svg "$HERE"/config.example.toml "$T/dist/packaging/"
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
  && [ -f "$T/home/.local/share/applications/fleet-board.desktop" ] && [ -f "$T/home/.local/share/icons/hicolor/scalable/apps/fleet-board.svg" ]; then
  pass installs_both_binaries_and_the_launcher_and_icon
else fail installs_both_binaries_and_the_launcher_and_icon "status $(status): $(cat "$T/out")"; fi

# 2
d="$T/home/.local/share/applications/fleet-board.desktop"
if grep -q "^Exec=bash -lc " "$d" && grep -q "$T/home/.local/bin/fleet-gui" "$d" && ! grep -q "@BIN@" "$d" && grep -q "^Icon=fleet-board" "$d" && grep -q "^StartupWMClass=fleet-board" "$d"; then
  pass launcher_exec_runs_through_a_login_shell
else fail launcher_exec_runs_through_a_login_shell "$(cat "$d" 2>&1)"; fi

# 3
if grep -q "^listen" "$T/home/.config/fleet-board/config.toml" 2>/dev/null || grep -q "listen" "$T/home/.config/fleet-board/config.toml" 2>/dev/null; then
  pass writes_the_config_template_when_absent
else fail writes_the_config_template_when_absent "no template written"; fi
[ -d "$T/home/.local/share/fleet-board/worktrees" ] && pass creates_the_worktrees_folder || fail creates_the_worktrees_folder "missing"

# 4
fixture; mkdir -p "$T/home/.config/fleet-board"; echo "# mine, do not touch" > "$T/home/.config/fleet-board/config.toml"
run_install
if [ "$(status)" = 0 ] && [ "$(cat "$T/home/.config/fleet-board/config.toml")" = "# mine, do not touch" ]; then pass never_overwrites_an_existing_config
else fail never_overwrites_an_existing_config "$(cat "$T/home/.config/fleet-board/config.toml")"; fi

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
grep -qi "trust this folder" "$T/out" && grep -q "worktrees" "$T/out" && pass prints_the_one_time_trust_step || fail prints_the_one_time_trust_step "$(cat "$T/out")"
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

# 10
fixture; run_install --prefix "$T/prefix"
[ "$(status)" = 0 ] && [ -x "$T/prefix/bin/fleet-daemon" ] && [ -x "$T/prefix/bin/fleet-gui" ] && pass honours_prefix || fail honours_prefix "$(cat "$T/out")"

echo
[ "$fails" = 0 ] && echo "all install tests passed" || echo "$fails install test(s) failed"
exit "$fails"
