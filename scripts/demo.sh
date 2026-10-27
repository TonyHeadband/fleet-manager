#!/usr/bin/env bash
# Demo media pipeline. It records one shot folder (--shot, default demo/shots/pie) in a throwaway world (own HOME,
# tmux socket, port, X display), so a checkout elsewhere can call it with its own shots: --shot <dir> --out <dir>.
# A shot directly under demo/shots/ is official: recorded with no override it also rewrites its line of demo/demo.lock.
# --check is the CI gate: it only compares the committed fingerprints, never records.
set -euo pipefail
# Relative --shot and --out are the caller's, not this repo's.
caller=$PWD
cd "$(dirname "$0")/.."
root=$PWD

# Tracked inputs shared by every shot. A shot adds its own folder and the paths in its lock-inputs file.
# Keep in sync with the docs-media paths in .fleet/workflow.yml.
COMMON_INPUTS=(crates/fleet-gui crates/fleet-client crates/fleet-api crates/fleet-demo crates/fleet-daemon/src/status.rs fixtures/hooks demo/bin scripts/demo.sh Cargo.lock)
LOCK=demo/demo.lock
SHOTS=demo/shots
FORMATS_ALL="webm gif png mp4"
GIF_LIMIT=$((5 * 1024 * 1024))

shot=$SHOTS/pie
out= size= length= still_at= formats=
all=0 scratch=0 shot_given=0
passthrough=()

# Paths a shot's media also depends on (relative to the repo root), one per line, read from the index like the rest.
lock_inputs() {
  git cat-file blob ":$SHOTS/$1/lock-inputs" 2>/dev/null | grep -v '^[[:space:]]*$' || true
}

# Index entries (mode, blob, path) rather than file contents: identical on every machine, ignores untracked files.
fingerprint() {
  local extra=()
  mapfile -t extra < <(lock_inputs "$1")
  git ls-files -s -- "${COMMON_INPUTS[@]}" "$SHOTS/$1" ${extra[@]+"${extra[@]}"} | sha256sum | cut -d' ' -f1
}

# The shots the lock covers, sorted.
official_shots() {
  local d
  for d in "$SHOTS"/*/; do
    [ -f "${d}shot.toml" ] && basename "$d"
  done
  return 0
}

# The folder's name when it sits directly under this repo's demo/shots/, else nothing.
official_name() {
  local dir
  dir=$(cd "$1" 2>/dev/null && pwd -P) || return 0
  [ "$(dirname "$dir")" = "$(cd "$SHOTS" 2>/dev/null && pwd -P)" ] && basename "$dir"
  return 0
}

lock_hash() {
  [ -f "$LOCK" ] || return 0
  awk -v n="$1" '$1 == n { print $2 }' "$LOCK"
}

# Rewrites the lines of the named shots (every official shot when none is named) and keeps the others.
write_lock() {
  local names=("$@") lines=() n
  if [ ${#names[@]} -eq 0 ]; then
    mapfile -t names < <(official_shots)
  elif [ -f "$LOCK" ]; then
    mapfile -t lines < <(grep -v '^#' "$LOCK" | awk -v names=" ${names[*]} " 'index(names, " " $1 " ") == 0')
  fi
  for n in ${names[@]+"${names[@]}"}; do
    lines+=("$n sha256:$(fingerprint "$n")")
  done
  mkdir -p "$(dirname "$LOCK")"
  {
    echo '# regenerate with scripts/demo.sh; do not edit'
    printf '%s\n' ${lines[@]+"${lines[@]}"} | sort
  } > "$LOCK"
}

# Only the pie gates CI; a stale short warns, since its media is recorded when its author asks for it.
check_lock() {
  local n rc=0
  for n in $(official_shots); do
    [ "$(lock_hash "$n")" = "sha256:$(fingerprint "$n")" ] && continue
    if [ "$n" = pie ]; then
      echo "demo media is stale, run scripts/demo.sh and commit docs/assets/ and demo/demo.lock" >&2
      rc=1
    else
      echo "warning: demo shot $n is stale, run scripts/demo.sh --shot $SHOTS/$n" >&2
    fi
  done
  return "$rc"
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
  "$bin/fleet-demo-director" --base-url "http://127.0.0.1:$port" --shot "$shot" \
    --cues "$W/cues" --work "$W" ${overrides[@]+"${overrides[@]}"} "$@" >"$W/director.out" 2>&1 &
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

# Error diffusion instead of Bayer: the ordered pattern shows as crosshatch over the UI's gradients. diff_mode
# re-encodes only the changed rectangle, which keeps the mostly static board small enough to stay at full width.
make_gif() {
  ffmpeg -nostdin -v error -y -i "$W/clip.mp4" -vf "$1,palettegen=stats_mode=diff" "$W/palette.png"
  ffmpeg -nostdin -v error -y -i "$W/clip.mp4" -i "$W/palette.png" \
    -lavfi "$1 [x]; [x][1:v] paletteuse=dither=sierra2_4a:diff_mode=rectangle" "$2"
}

# The value of one `KEY value` line of the director's --describe.
described() {
  sed -nE "s/^$1 (.*)$/\1/p" <<< "$desc"
}

has_format() {
  [[ " $formats " == *" $1 "* ]]
}

record() {
  local name
  name=$(official_name "$shot")
  if [ -z "$name" ] && [ -z "$out" ]; then
    echo "--out is required for a shot outside demo/shots" >&2
    exit 2
  fi
  [ -f "$shot/shot.toml" ] || { echo "no $shot/shot.toml" >&2; exit 1; }
  # Absolute: the agent runs in each task's own folder.
  shot=$(cd "$shot" && pwd)
  need_tools

  local overrides=()
  [ -z "$length" ] || overrides+=(--length "$length")
  [ -z "$still_at" ] || overrides+=(--still-at "$still_at")

  cargo build --release -p fleet-daemon -p fleet-gui -p fleet-demo
  local bin=$root/target/release

  # Resolved by the director, so the shot's own file is the only place the settings are parsed.
  local desc
  desc=$("$bin/fleet-demo-director" --shot "$shot" ${overrides[@]+"${overrides[@]}"} --describe)
  length=$(described LENGTH)
  still_at=$(described STILL_AT)
  [ -n "$size" ] || size=$(described SIZE)
  [ -n "$formats" ] || formats=$(described FORMATS)
  local stem still
  stem=$(described STEM)
  still=$(described STILL)
  if [ -z "$out" ]; then
    out=$(described OUT_DIR)
    [ -n "$out" ] || { echo "$shot/shot.toml has no [output] dir; pass --out" >&2; exit 1; }
    out=$root/$out
  fi
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
claude_command = ["$bin/fleet-demo-agent", "--shot", "$shot"]
gh_command = "$root/demo/bin/gh"
EOF
  cat > "$HOME/.config/fleet-manager/gui.toml" <<EOF
theme = "dark"
close_choice = "keep_running"
EOF

  Xvfb ":$display" -screen 0 "${size}x24" +extension GLX -nolisten tcp >"$W/xvfb.log" 2>&1 &
  XVFB_PID=$!
  wait_for "Xvfb :$display" 10 test -e "/tmp/.X11-unix/X$display"
  export DISPLAY=:$display

  "$bin/fleet-daemon" >"$W/daemon.out" 2>&1 &
  DAEMON_PID=$!
  wait_for "the daemon on port $port" 15 eval "ss -ltnH 'sport = :$port' | grep -q ."

  # Before the GUI starts: with no ready persona it would open its first-persona wizard over the board. Naming the
  # persona in gui.toml also spares the "Settings saved" toast the GUI shows when it picks one itself.
  direct --setup
  local persona
  persona=$(sed -nE 's/^PERSONA (.+)$/\1/p' "$W/director.out")
  [ -n "$persona" ] || { echo "the director named no persona" >&2; exit 1; }
  local product
  product=$(sed -nE 's/^PRODUCT (.+)$/\1/p' "$W/director.out")
  [ -n "$product" ] || { echo "the director named no product" >&2; exit 1; }
  # The product is selected here rather than by a click, so the recording opens on its board.
  printf 'active_persona = "%s"\nactive_product = "%s"\n' "$persona" "$product" >> "$HOME/.config/fleet-manager/gui.toml"

  # The daemon is already up, so the GUI connects to it instead of starting its own.
  FLEET_GUI_CUES=$W/cues FLEET_GUI_SIZE=$size LIBGL_ALWAYS_SOFTWARE=1 "$bin/fleet-gui" >"$W/gui.out" 2>&1 &
  GUI_PID=$!
  wait_for "the Fleet Manager window" 30 eval "xwininfo -root -tree | grep -q '\"Fleet Manager\"'"
  local geometry
  geometry=$(xwininfo -root -tree | grep '"Fleet Manager"' | grep -oE ' [0-9]+x[0-9]+\+' | head -1 | tr -d ' +' || true)
  if [ "$geometry" != "$size" ]; then
    echo "the window is $geometry, expected $size" >&2
    exit 1
  fi
  # Lets the first snapshot arrive, so the recording opens on a live, empty board.
  sleep 2

  # No pointer: nothing moves it, so it would sit as a stray cursor in the middle of every frame.
  ffmpeg -nostdin -hide_banner -f x11grab -draw_mouse 0 -framerate 30 -video_size "$size" -i ":$display" \
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

  mkdir -p "$out"
  ffmpeg -nostdin -v error -y -ss "$offset" -i "$W/raw.mp4" -t "$length" -c:v libx264 -crf 18 "$W/clip.mp4"
  local made=()
  # The clip is already trimmed and encoded, so mp4 is a copy.
  if has_format mp4; then
    cp "$W/clip.mp4" "$out/$stem.mp4"
    made+=("$out/$stem.mp4")
  fi
  if has_format webm; then
    ffmpeg -nostdin -v error -y -i "$W/clip.mp4" -c:v libvpx-vp9 -crf 34 -b:v 0 -an "$out/$stem.webm"
    made+=("$out/$stem.webm")
  fi
  if has_format png; then
    ffmpeg -nostdin -v error -y -ss "$still_at" -i "$W/clip.mp4" -frames:v 1 "$out/$still.png"
    made+=("$out/$still.png")
  fi
  if has_format gif; then
    # Native size first: any downscale blurs the UI's small text. Shrink only to get under the limit.
    local filters
    for filters in "fps=15" "fps=12" "fps=12,scale=1200:-1:flags=lanczos" "fps=10,scale=1000:-1:flags=lanczos"; do
      make_gif "$filters" "$out/$stem.gif"
      if [ "$(stat -c %s "$out/$stem.gif")" -le "$GIF_LIMIT" ]; then break; fi
    done
    made+=("$out/$stem.gif")
  fi

  # Any flag that changes the render makes it a scratch copy, which must not mark the official media fresh.
  if [ -n "$name" ] && [ "$scratch" -eq 0 ]; then
    write_lock "$name"
  fi
  local f
  for f in "${made[@]}"; do
    echo "$f $(du -h "$f" | cut -f1)"
  done
}

# Every official shot in turn, each in its own world and to its own [output] dir.
record_all() {
  local n
  for n in $(official_shots); do
    bash "$root/scripts/demo.sh" --shot "$SHOTS/$n" ${passthrough[@]+"${passthrough[@]}"}
  done
}

usage() {
  cat >&2 <<'EOF'
usage: scripts/demo.sh [--shot DIR] [--out DIR] [--size WxH] [--length D] [--still-at D] [--formats LIST]
       scripts/demo.sh --all | --check | --lock [--shot DIR]
EOF
  exit 2
}

absolute() {
  case "$1" in
    /*) printf '%s' "$1" ;;
    *) printf '%s/%s' "$caller" "$1" ;;
  esac
}

mode=record
while [ $# -gt 0 ]; do
  case "$1" in
    --shot|--out|--size|--length|--still-at|--formats) [ $# -ge 2 ] || usage ;;
  esac
  case "$1" in
    --shot) shot=$(absolute "$2"); shot_given=1; shift 2; continue ;;
    # Resolved here and handed on resolved: --all re-runs this script from the repo root, where a relative path
    # would mean something else.
    --out) out=$(absolute "$2"); set -- "$1" "$out" "${@:3}" ;;
    --size) size=$2 ;;
    --length) length=$2 ;;
    --still-at) still_at=$2 ;;
    --formats)
      formats=${2//,/ }
      [ -n "${formats// /}" ] || { echo "--formats needs at least one of $FORMATS_ALL" >&2; exit 2; }
      ;;
    --all) all=1; shift; continue ;;
    --check) mode=check; shift; continue ;;
    --lock) mode=lock; shift; continue ;;
    *) usage ;;
  esac
  # Not --shot, --all, --check or --lock: these change the render, and --all hands them on to each shot.
  scratch=1
  passthrough+=("$1" "$2")
  shift 2
done

if [[ -n "$size" && ! "$size" =~ ^[0-9]+x[0-9]+$ ]]; then
  echo "--size $size is not WxH" >&2
  exit 2
fi
for f in $formats; do
  [[ " $FORMATS_ALL " == *" $f "* ]] || { echo "--formats: $f is not one of $FORMATS_ALL" >&2; exit 2; }
done

case "$mode" in
  check) check_lock ;;
  lock)
    if [ "$shot_given" -eq 1 ]; then
      name=$(official_name "$shot")
      [ -n "$name" ] || { echo "$shot is not under demo/shots, which is all the lock covers" >&2; exit 2; }
      write_lock "$name"
    else
      write_lock
    fi
    ;;
  record)
    if [ "$all" -eq 1 ]; then
      [ "$shot_given" -eq 0 ] || usage
      record_all
    else
      record
    fi
    ;;
esac
