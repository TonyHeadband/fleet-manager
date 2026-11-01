#!/usr/bin/env bash
# The egui app and the web UI side by side on one throwaway daemon, for comparing the two. The world is demo.sh's
# (own HOME, port and state, the scripted demo agent instead of claude), so the owner's fleet is never touched.
# The demo timeline plays once to fill the board. --lan serves the web UI to the local network as well: anyone who
# can reach this machine can then drive the throwaway daemon, which only ever runs the demo agent.
source "$(dirname "$0")/lib.sh"

host=127.0.0.1
case "${1:-}" in
  --lan) host=0.0.0.0 ;;
  "") ;;
  *) echo "usage: scripts/ui-compare.sh [--lan]" >&2; exit 2 ;;
esac

root=$(pwd)
real_home=$HOME
cargo test -q -p fleet-gui --test theme_css
cargo build -p fleet-daemon -p fleet-gui -p fleet-demo
bin=$root/$TARGET_DIR/debug
pnpm --dir web install --frozen-lockfile >/dev/null

port=7990
while ss -ltnH "sport = :$port" | grep -q .; do port=$((port + 1)); done

W=$(mktemp -d /tmp/fleet-compare.XXXXXX)
PIDS=()
cleanup() {
  local pid
  for pid in "${PIDS[@]}"; do kill -TERM "$pid" 2>/dev/null || true; done
  wait 2>/dev/null || true
  # A daemon the GUI restarted on its own names this run's folder in its --config argument.
  pkill -f -- "${W//./\\.}/" 2>/dev/null || true
  rm -rf "$W"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

export HOME=$W/home
mkdir -p "$HOME/.config/fleet-manager" "$W/tasks" "$W/state"
cat > "$HOME/.config/fleet-manager/config.toml" <<EOF
listen = "127.0.0.1:$port"
tasks_root = "$W/tasks"
state_dir = "$W/state"
claude_command = ["$bin/fleet-demo-agent", "--scenarios", "$root/demo/scenarios"]
gh_command = "$root/demo/bin/gh"
EOF
echo 'close_choice = "keep_running"' > "$HOME/.config/fleet-manager/gui.toml"

git init -q "$W/bakery"
echo "Recipes for the bakery." > "$W/bakery/README.txt"
git -C "$W/bakery" add README.txt
git -C "$W/bakery" -c user.name=Baker -c user.email=baker@pie.example commit -q -m "Start the bakery"

# The persona's login check asks GitHub who the token is: the stand-in answers baker.
"$bin/fleet-demo-forge" --addr-file "$W/forge.addr" >"$W/forge.out" 2>&1 &
PIDS+=($!)
for _ in $(seq 50); do [ -s "$W/forge.addr" ] && break; sleep 0.1; done
FLEET_FORGE_API=$(cat "$W/forge.addr")
export FLEET_FORGE_API

"$bin/fleet-daemon" >"$W/daemon.out" 2>&1 &
PIDS+=($!)
for _ in $(seq 150); do
  ss -ltnH "sport = :$port" | grep -q . && break
  sleep 0.1
done
ss -ltnH "sport = :$port" | grep -q . || { cat "$W/daemon.out" >&2; echo "the daemon did not start" >&2; exit 1; }

director() {
  "$bin/fleet-demo-director" --base-url "http://127.0.0.1:$port" --timeline demo/timeline.toml \
    --cues "$W/cues" --repo "$W/bakery" "$@"
}
# A persona and product before the GUI starts, so it opens on the board rather than its first-persona wizard.
director --persona-only >"$W/director.out"
persona=$(sed -nE 's/^PERSONA (.+)$/\1/p' "$W/director.out")
product=$(sed -nE 's/^PRODUCT (.+)$/\1/p' "$W/director.out")
printf 'active_persona = "%s"\nactive_product = "%s"\n' "$persona" "$product" >> "$HOME/.config/fleet-manager/gui.toml"

"$bin/fleet-gui" >"$W/gui.out" 2>&1 &
PIDS+=($!)

# Vite itself rather than `pnpm dev`: pnpm does not pass TERM on, which would leave the server running.
(cd web && HOME=$real_home FLEET_DAEMON_URL="http://127.0.0.1:$port" exec node_modules/.bin/vite --host "$host" --port 5173 --strictPort) &
PIDS+=($!)

director --persona "$persona" --product "$product" >"$W/timeline.out" 2>&1 &
PIDS+=($!)

echo "daemon on 127.0.0.1:$port; the web UI is at the address vite printed above. Ctrl-C stops everything."
wait "${PIDS[0]}"
