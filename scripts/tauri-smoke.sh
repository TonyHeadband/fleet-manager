#!/usr/bin/env bash
# The Tauri app's smoke test, as CI runs it on Ubuntu and Windows: a throwaway home with a daemon up and a ready persona
# (the demo's stand-ins for claude, gh and GitHub's API, as scripts/demo.sh), then WebdriverIO drives the release app
# through tauri-driver: under Xvfb with WebKitWebDriver on Linux, with msedgedriver (WebView2) on Windows, in Git Bash,
# named by FLEET_NATIVE_DRIVER. Needs: the release binaries (fleet-gui-tauri, fleet-daemon, fleet-demo-*),
# tauri-driver, and xvfb-run with WebKitWebDriver on Linux. Your own daemon and settings are never read or touched.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
bin=$root/target/release

case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) windows=1 exe=.exe ;;
  *) windows= exe= ;;
esac
# A path as the OS's own programs take it: `C:/…` on Windows (TOML and every Windows program read `/`).
native() { if [ -n "$windows" ]; then cygpath -m "$1"; else printf '%s\n' "$1"; fi; }
# True when something accepts connections on the loopback port.
listening() { (exec 3<> "/dev/tcp/127.0.0.1/$1") 2> /dev/null; }

# When the run began, in local time as Windows reports process starts: the cleanup leaves anything older alone.
started=$(date '+%Y-%m-%dT%H:%M:%S.%3N')
W=$(mktemp -d "${TMPDIR:-/tmp}/fleet-smoke.XXXXXX")
DAEMON_PID= FORGE_PID=
cleanup() {
  local rc=$?
  # The test's own status is the script's: nothing below may change it.
  set +e
  # A failed run shows what the daemon and the app said: the test itself only sees the page.
  if [ "$rc" != 0 ]; then
    for f in "$W/daemon.out" "$W/state/gui.log"; do
      [ -s "$f" ] && { echo "== $f" >&2; tail -n 60 "$f" >&2; }
    done
  fi
  [ -z "$DAEMON_PID" ] || kill "$DAEMON_PID" 2>/dev/null || true
  [ -z "$FORGE_PID" ] || kill "$FORGE_PID" 2>/dev/null || true
  if [ -n "$windows" ]; then
    # No pkill in Git Bash: what this run started, so only processes created since it began, and of those the ones
    # naming its folder (either spelling) or run from this build. Your own app or daemon from target\release is older.
    W_WIN=$(cygpath -w "$W")\\ W_MIX=$(cygpath -m "$W")/ BIN_DIR=$(cygpath -w "$bin")\\ SINCE=$started \
      powershell.exe -NoProfile -Command '
        $since = [datetime]::Parse($env:SINCE, [Globalization.CultureInfo]::InvariantCulture)
        Get-CimInstance Win32_Process | Where-Object { $_.CreationDate -ge $since -and (
            ($_.CommandLine -and ($_.CommandLine.IndexOf($env:W_WIN, [StringComparison]::OrdinalIgnoreCase) -ge 0 -or
              $_.CommandLine.IndexOf($env:W_MIX, [StringComparison]::OrdinalIgnoreCase) -ge 0)) -or
            ($_.ExecutablePath -and $_.ExecutablePath.StartsWith($env:BIN_DIR, [StringComparison]::OrdinalIgnoreCase))) } |
          ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }' 2>/dev/null
  else
    # Anything still naming this run's folder; the escaped dot keeps the pattern to it.
    local pattern="${W//./\\.}/"
    pkill -f -- "$pattern" 2>/dev/null || true
  fi
  # Let them exit before the folder goes: a late write (the app's Mesa shader cache) makes rm fail on a non-empty dir.
  [ -z "$DAEMON_PID" ] || wait "$DAEMON_PID" 2>/dev/null || true
  [ -z "$FORGE_PID" ] || wait "$FORGE_PID" 2>/dev/null || true
  [ -n "$windows" ] || for _ in $(seq 100); do pgrep -f -- "$pattern" > /dev/null 2>&1 || break; sleep 0.1; done
  for _ in $(seq 10); do rm -rf "$W" 2>/dev/null && [ ! -e "$W" ] && break; sleep 0.5; done || true
  exit "$rc"
}
trap cleanup EXIT

port=7990
while listening "$port"; do port=$((port + 1)); done

home=$W/home
export HOME=$home
# Windows programs take the home from USERPROFILE (fleet_fs::dirs::home_dir), and keep their config under AppData.
if [ -n "$windows" ]; then
  USERPROFILE=$(cygpath -w "$home")
  export USERPROFILE
  config=$home/AppData/Roaming/fleet-manager
else
  config=$home/.config/fleet-manager
fi
# No Mesa shader cache under the temp HOME: it is written as the app exits and races the cleanup.
export MESA_SHADER_CACHE_DISABLE=true
mkdir -p "$config" "$W/tasks" "$W/state"
# Literal strings: a Windows path's `\` is no escape in them.
cat > "$config/config.toml" <<TOML
listen = '127.0.0.1:$port'
tasks_root = '$(native "$W/tasks")'
state_dir = '$(native "$W/state")'
claude_command = ['$(native "$bin/fleet-demo-agent$exe")', '--scenarios', '$(native "$root/demo/scenarios")']
gh_command = '$(native "$bin/fleet-demo-gh$exe")'
TOML
printf 'close_choice = "keep_running"\n' > "$config/gui.toml"
git init -q "$W/bakery"
echo "Recipes for the bakery." > "$W/bakery/README.txt"
git -C "$W/bakery" add README.txt
git -C "$W/bakery" -c user.name=Baker -c user.email=baker@pie.example commit -q -m "Start the bakery"

# The persona's login check asks GitHub who the token is: the stand-in answers baker.
"$bin/fleet-demo-forge$exe" --addr-file "$(native "$W/forge.addr")" > "$W/forge.out" 2>&1 &
FORGE_PID=$!
for _ in $(seq 50); do [ -s "$W/forge.addr" ] && break; sleep 0.1; done
FLEET_FORGE_API=$(cat "$W/forge.addr")
export FLEET_FORGE_API

"$bin/fleet-daemon$exe" > "$W/daemon.out" 2>&1 &
DAEMON_PID=$!
for _ in $(seq 150); do listening "$port" && break; sleep 0.1; done
"$bin/fleet-demo-director$exe" --base-url "http://127.0.0.1:$port" --timeline "$(native "$root/demo/timeline.toml")" \
  --cues "$(native "$W/cues")" --repo "$(native "$W/bakery")" --persona-only > "$W/director.out" 2>&1 || {
  cat "$W/director.out" "$W/daemon.out" >&2
  exit 1
}

FLEET_TAURI_APP=$(native "$bin/fleet-gui-tauri$exe")
export FLEET_TAURI_APP
cd "$root/web"
if [ -n "$windows" ]; then
  pnpm exec wdio run wdio.conf.ts
else
  xvfb-run -a pnpm exec wdio run wdio.conf.ts
fi
