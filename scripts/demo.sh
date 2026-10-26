#!/usr/bin/env bash
# Demo media pipeline. No argument records docs/assets/ in a throwaway world (own HOME, tmux socket, port, X display)
# and rewrites demo/demo.lock. --check is the CI gate: it only compares the committed fingerprint, never records.
set -euo pipefail
cd "$(dirname "$0")/.."

# Tracked inputs that change what the demo shows. Keep in sync with the docs-media paths in .fleet/workflow.yml.
DEMO_INPUTS=(crates/fleet-gui crates/fleet-client crates/fleet-api crates/fleet-demo crates/fleet-daemon/src/status.rs fixtures/hooks demo/scenarios demo/timeline.toml demo/bin scripts/demo.sh Cargo.lock)
LOCK=demo/demo.lock
ASSETS=docs/assets
SIZE=1400x800
GIF_LIMIT=$((5 * 1024 * 1024))

# Index entries (mode, blob, path) rather than file contents: identical on every machine, ignores untracked files.
fingerprint() {
  git ls-files -s -- "${DEMO_INPUTS[@]}" | sha256sum | cut -d' ' -f1
}

write_lock() {
  mkdir -p "$(dirname "$LOCK")"
  printf '# regenerate with scripts/demo.sh; do not edit\nsha256:%s\n' "$(fingerprint)" > "$LOCK"
}

check_lock() {
  if [ -f "$LOCK" ] && [ "$(sed -n 2p "$LOCK")" = "sha256:$(fingerprint)" ]; then
    return 0
  fi
  echo "demo media is stale, run scripts/demo.sh and commit docs/assets/ and demo/demo.lock" >&2
  return 1
}

need_tools() {
  local missing=() t
  for t in Xvfb ffmpeg tmux git xwininfo cargo sha256sum ss awk seq; do
    command -v "$t" >/dev/null || missing+=("$t")
  done
  if [ ${#missing[@]} -gt 0 ]; then
    echo "missing: ${missing[*]}" >&2
    exit 1
  fi
  local v
  v=$(tmux -V | sed -E 's/^tmux (next-)?([0-9]+\.[0-9]+).*/\2/')
  if ! printf '3.2\n%s\n' "$v" | sort -C -V; then
    echo "tmux $v is too old: the daemon needs 3.2 or newer" >&2
    exit 1
  fi
}

# "25s" or "1500ms" (the timeline's duration syntax) as seconds.
seconds() {
  case "$1" in
    *ms) awk -v n="${1%ms}" 'BEGIN { printf "%.3f", n / 1000 }' ;;
    *s) awk -v n="${1%s}" 'BEGIN { printf "%.3f", n }' ;;
    *) echo "bad duration $1" >&2; exit 1 ;;
  esac
}

timeline_value() {
  local v
  v=$(sed -nE "s/^$1 *= *\"([^\"]+)\".*/\1/p" demo/timeline.toml)
  [ -n "$v" ] || { echo "demo/timeline.toml has no $1" >&2; exit 1; }
  seconds "$v"
}

free_port() {
  local p=7990
  while ss -ltnH "sport = :$p" | grep -q .; do p=$((p + 1)); done
  echo "$p"
}

free_display() {
  local d=90
  while [ -e "/tmp/.X$d-lock" ] || [ -e "/tmp/.X11-unix/X$d" ]; do d=$((d + 1)); done
  echo "$d"
}

# Polls a condition every 0.1 s and names what it waited for when it gives up.
wait_for() {
  local what=$1 tries=$(($2 * 10))
  shift 2
  until "$@"; do
    tries=$((tries - 1))
    if [ "$tries" -le 0 ]; then
      echo "timed out waiting for $what" >&2
      exit 1
    fi
    sleep 0.1
  done
}

# TERM, then KILL if it lingers. ffmpeg finalises its file on TERM as it does on q or INT, and INT would not reach it:
# background jobs of a non-interactive shell ignore SIGINT.
stop_pid() {
  local pid=$1 i
  [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null || return 0
  kill -TERM "$pid" 2>/dev/null || true
  for i in $(seq 50); do
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.1
  done
  kill -KILL "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
}

FFMPEG_PID= DIRECTOR_PID= GUI_PID= DAEMON_PID= XVFB_PID= W= CLEANED=
# Only what this run started, by recorded PID and its own tmux socket: the owner's daemon and tmux are never touched.
cleanup() {
  [ -z "$CLEANED" ] || return 0
  CLEANED=1
  stop_pid "$FFMPEG_PID"
  stop_pid "$DIRECTOR_PID"
  stop_pid "$GUI_PID"
  stop_pid "$DAEMON_PID"
  stop_pid "$XVFB_PID"
  if [ -n "${FLEET_TMUX_SOCKET:-}" ]; then
    tmux -L "$FLEET_TMUX_SOCKET" kill-server 2>/dev/null || true
  fi
  if [ -n "$W" ]; then
    # Anything still naming the run's folder, such as a daemon the GUI restarted on its own (it passes
    # --config $W/home/...). The trailing slash and escaped dot keep the pattern to this run's unique folder.
    pkill -f -- "${W//./\\.}/" 2>/dev/null || true
    rm -rf "$W"
  fi
}

# Runs the director in the background, so a signal to this script is handled at once instead of after the timeline.
direct() {
  "$bin/fleet-demo-director" --base-url "http://127.0.0.1:$port" --timeline demo/timeline.toml \
    --cues "$W/cues" --repo "$W/bakery" "$@" >"$W/director.out" 2>&1 &
  DIRECTOR_PID=$!
  if ! wait "$DIRECTOR_PID"; then
    DIRECTOR_PID=
    cat "$W/director.out" >&2
    echo "the director failed; the end of the daemon log:" >&2
    tail -n 40 "$W/state/daemon.log" >&2 || true
    exit 1
  fi
  DIRECTOR_PID=
}

make_gif() {
  ffmpeg -nostdin -v error -y -i "$W/clip.mp4" -vf "$1,palettegen=stats_mode=diff" "$W/palette.png"
  ffmpeg -nostdin -v error -y -i "$W/clip.mp4" -i "$W/palette.png" \
    -lavfi "$1 [x]; [x][1:v] paletteuse=dither=bayer:bayer_scale=4" "$ASSETS/demo.gif"
}

record() {
  need_tools
  local length still_at root
  length=$(timeline_value length)
  still_at=$(timeline_value still_at)
  root=$(pwd)

  cargo build --release -p fleet-daemon -p fleet-gui -p fleet-demo
  local bin=$root/target/release

  # Short on purpose: the tmux socket lives under it, and a Unix socket path must stay under ~108 bytes.
  W=$(mktemp -d /tmp/fleet-demo.XXXXXX)
  trap cleanup EXIT
  # Exiting runs the EXIT trap. Every long wait below is on a background job, so a signal cuts it short.
  trap 'exit 130' INT
  trap 'exit 143' TERM

  export HOME=$W/home TMUX_TMPDIR=$W FLEET_TMUX_SOCKET=fleet-demo-$$
  local port display
  port=$(free_port)
  display=$(free_display)
  mkdir -p "$HOME/.config/fleet-manager" "$W/tasks" "$W/state"
  cat > "$HOME/.config/fleet-manager/config.toml" <<EOF
listen = "127.0.0.1:$port"
tasks_root = "$W/tasks"
state_dir = "$W/state"
claude_command = ["$bin/fleet-demo-agent", "--scenarios", "$root/demo/scenarios"]
gh_command = "$root/demo/bin/gh"
EOF
  cat > "$HOME/.config/fleet-manager/gui.toml" <<EOF
theme = "dark"
close_choice = "keep_running"
EOF

  git init -q "$W/bakery"
  echo "Recipes for the bakery." > "$W/bakery/README.txt"
  git -C "$W/bakery" add README.txt
  git -C "$W/bakery" -c user.name=Baker -c user.email=baker@pie.example commit -q -m "Start the bakery"

  Xvfb ":$display" -screen 0 "${SIZE}x24" +extension GLX -nolisten tcp >"$W/xvfb.log" 2>&1 &
  XVFB_PID=$!
  wait_for "Xvfb :$display" 10 test -e "/tmp/.X11-unix/X$display"
  export DISPLAY=:$display

  "$bin/fleet-daemon" >"$W/daemon.out" 2>&1 &
  DAEMON_PID=$!
  wait_for "the daemon on port $port" 15 eval "ss -ltnH 'sport = :$port' | grep -q ."

  # Before the GUI starts: with no ready persona it would open its first-persona wizard over the board. Naming the
  # persona in gui.toml also spares the "Settings saved" toast the GUI shows when it picks one itself.
  direct --persona-only
  local persona
  persona=$(sed -nE 's/^PERSONA (.+)$/\1/p' "$W/director.out")
  [ -n "$persona" ] || { echo "the director named no persona" >&2; exit 1; }
  local product
  product=$(sed -nE 's/^PRODUCT (.+)$/\1/p' "$W/director.out")
  [ -n "$product" ] || { echo "the director named no product" >&2; exit 1; }
  # The product is selected here rather than by a click, so the recording opens on its board.
  printf 'active_persona = "%s"\nactive_product = "%s"\n' "$persona" "$product" >> "$HOME/.config/fleet-manager/gui.toml"

  # The daemon is already up, so the GUI connects to it instead of starting its own.
  FLEET_GUI_CUES=$W/cues LIBGL_ALWAYS_SOFTWARE=1 "$bin/fleet-gui" >"$W/gui.out" 2>&1 &
  GUI_PID=$!
  wait_for "the Fleet Manager window" 30 eval "xwininfo -root -tree | grep -q '\"Fleet Manager\"'"
  local geometry
  geometry=$(xwininfo -root -tree | grep '"Fleet Manager"' | grep -oE ' [0-9]+x[0-9]+\+' | head -1 | tr -d ' +' || true)
  if [ "$geometry" != "$SIZE" ]; then
    echo "the window is $geometry, expected $SIZE" >&2
    exit 1
  fi
  # Lets the first snapshot arrive, so the recording opens on a live, empty board.
  sleep 2

  # No pointer: nothing moves it, so it would sit as a stray cursor in the middle of every frame.
  ffmpeg -nostdin -hide_banner -f x11grab -draw_mouse 0 -framerate 30 -video_size "$SIZE" -i ":$display" \
    -c:v libx264 -crf 18 "$W/raw.mp4" >"$W/ffmpeg.log" 2>&1 &
  FFMPEG_PID=$!
  wait_for "ffmpeg to start capturing" 10 grep -q 'start: [0-9]' "$W/ffmpeg.log"

  direct --persona "$persona" --product "$product"
  stop_pid "$FFMPEG_PID"
  FFMPEG_PID=

  # x11grab stamps frames with the wall clock, so the capture's start and the director's READY share one clock.
  local capture_start ready offset
  capture_start=$(sed -nE 's/.*start: ([0-9.]+).*/\1/p' "$W/ffmpeg.log" | head -1)
  ready=$(sed -nE 's/^READY ([0-9]+)$/\1/p' "$W/director.out")
  if [ -z "$capture_start" ] || [ -z "$ready" ]; then
    echo "cannot line up the capture: ffmpeg start '$capture_start', director READY '$ready'" >&2
    exit 1
  fi
  offset=$(awk -v r="$ready" -v s="$capture_start" 'BEGIN { printf "%.3f", r / 1000 - s }')

  mkdir -p "$ASSETS"
  ffmpeg -nostdin -v error -y -ss "$offset" -i "$W/raw.mp4" -t "$length" -c:v libx264 -crf 18 "$W/clip.mp4"
  ffmpeg -nostdin -v error -y -i "$W/clip.mp4" -c:v libvpx-vp9 -crf 34 -b:v 0 -an "$ASSETS/demo.webm"
  ffmpeg -nostdin -v error -y -ss "$still_at" -i "$W/clip.mp4" -frames:v 1 "$ASSETS/board.png"
  make_gif "fps=12,scale=1000:-1:flags=lanczos"
  if [ "$(stat -c %s "$ASSETS/demo.gif")" -gt "$GIF_LIMIT" ]; then
    make_gif "fps=10,scale=900:-1:flags=lanczos"
  fi

  write_lock
  local f
  for f in "$ASSETS/board.png" "$ASSETS/demo.gif" "$ASSETS/demo.webm"; do
    echo "$f $(du -h "$f" | cut -f1)"
  done
}

case "${1:-}" in
  --check) check_lock ;;
  --lock) write_lock ;;
  "") record ;;
  *) echo "usage: scripts/demo.sh [--check|--lock]" >&2; exit 2 ;;
esac
