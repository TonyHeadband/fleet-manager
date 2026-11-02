#!/usr/bin/env bash
# The Tauri app's smoke test, as CI runs it on Ubuntu: a throwaway HOME with a daemon up and a ready persona (the
# demo's stand-ins for claude, gh and GitHub's API, as scripts/demo.sh), then WebdriverIO drives the release app through
# tauri-driver under Xvfb. Needs: the release binaries (fleet-gui-tauri, fleet-daemon, fleet-demo-*), xvfb-run,
# tauri-driver and WebKitWebDriver. Your own daemon and settings are never read or touched.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
bin=$root/target/release

W=$(mktemp -d /tmp/fleet-smoke.XXXXXX)
DAEMON_PID= FORGE_PID=
cleanup() {
  local rc=$?
  # The test's own status is the script's: nothing below may change it.
  set +e
  [ -z "$DAEMON_PID" ] || kill "$DAEMON_PID" 2>/dev/null || true
  [ -z "$FORGE_PID" ] || kill "$FORGE_PID" 2>/dev/null || true
  # Anything still naming this run's folder; the escaped dot keeps the pattern to it.
  local pattern="${W//./\\.}/"
  pkill -f -- "$pattern" 2>/dev/null || true
  # Let them exit before the folder goes: a late write (the app's Mesa shader cache) makes rm fail on a non-empty dir.
  [ -z "$DAEMON_PID" ] || wait "$DAEMON_PID" 2>/dev/null || true
  [ -z "$FORGE_PID" ] || wait "$FORGE_PID" 2>/dev/null || true
  for _ in $(seq 100); do pgrep -f -- "$pattern" > /dev/null 2>&1 || break; sleep 0.1; done
  for _ in $(seq 5); do rm -rf "$W" 2>/dev/null && [ ! -e "$W" ] && break; sleep 0.5; done || true
  exit "$rc"
}
trap cleanup EXIT

port=7990
while ss -ltnH "sport = :$port" | grep -q .; do port=$((port + 1)); done

export HOME=$W/home
# No Mesa shader cache under the temp HOME: it is written as the app exits and races the cleanup.
export MESA_SHADER_CACHE_DISABLE=true
mkdir -p "$HOME/.config/fleet-manager" "$W/tasks" "$W/state"
cat > "$HOME/.config/fleet-manager/config.toml" <<TOML
listen = "127.0.0.1:$port"
tasks_root = "$W/tasks"
state_dir = "$W/state"
claude_command = ["$bin/fleet-demo-agent", "--scenarios", "$root/demo/scenarios"]
gh_command = "$root/demo/bin/gh"
TOML
printf 'close_choice = "keep_running"\n' > "$HOME/.config/fleet-manager/gui.toml"
git init -q "$W/bakery"
echo "Recipes for the bakery." > "$W/bakery/README.txt"
git -C "$W/bakery" add README.txt
git -C "$W/bakery" -c user.name=Baker -c user.email=baker@pie.example commit -q -m "Start the bakery"

# The persona's login check asks GitHub who the token is: the stand-in answers baker.
"$bin/fleet-demo-forge" --addr-file "$W/forge.addr" > "$W/forge.out" 2>&1 &
FORGE_PID=$!
for _ in $(seq 50); do [ -s "$W/forge.addr" ] && break; sleep 0.1; done
FLEET_FORGE_API=$(cat "$W/forge.addr")
export FLEET_FORGE_API

"$bin/fleet-daemon" > "$W/daemon.out" 2>&1 &
DAEMON_PID=$!
for _ in $(seq 150); do ss -ltnH "sport = :$port" | grep -q . && break; sleep 0.1; done
"$bin/fleet-demo-director" --base-url "http://127.0.0.1:$port" --timeline "$root/demo/timeline.toml" \
  --cues "$W/cues" --repo "$W/bakery" --persona-only > "$W/director.out" 2>&1 || {
  cat "$W/director.out" "$W/daemon.out" >&2
  exit 1
}

export FLEET_TAURI_APP=$bin/fleet-gui-tauri
cd "$root/web"
xvfb-run -a pnpm exec wdio run wdio.conf.ts
