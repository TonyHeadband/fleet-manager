# Shared setup for build.sh and scripts/*.sh. Sourced, not run.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

GLIBC="${FLEET_GLIBC:-2.28}"
TARGETS="${FLEET_TARGETS:-x86_64 aarch64}"
TARGET_DIR="${CARGO_TARGET_DIR:-target}"

# Only the scripts that cross-build call this, so web.sh and test-bundle.sh run without zig.
need_zig() {
  if [ -z "${CARGO_ZIGBUILD_ZIG_PATH:-}" ] && ! command -v zig >/dev/null; then
    # ~/.cache/fleet-board is the venv made before the rename; a venv cannot be moved, so look there too.
    local candidate
    candidate="$(ls "$HOME"/.cache/fleet-manager/zig-venv/lib/python*/site-packages/ziglang/zig "$HOME"/.cache/fleet-board/zig-venv/lib/python*/site-packages/ziglang/zig 2>/dev/null | head -1 || true)"
    [ -n "$candidate" ] || { echo "zig not found. Install one: python3 -m venv ~/.cache/fleet-manager/zig-venv && ~/.cache/fleet-manager/zig-venv/bin/pip install ziglang" >&2; exit 1; }
    export CARGO_ZIGBUILD_ZIG_PATH="$candidate"
  fi
  command -v cargo-zigbuild >/dev/null || { echo "cargo-zigbuild not found: cargo install --locked cargo-zigbuild" >&2; exit 1; }
}
