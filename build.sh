#!/usr/bin/env bash
# Builds the web UI, embeds it, and produces fleet-daemon for Linux x86_64 and aarch64 in ./dist.
# Binaries are linked against glibc 2.28 (the floor on Rocky/RHEL 8) through zig, so one build runs everywhere above it.
# Needs: rust + cargo-zigbuild, a zig (`pip install ziglang`), node 20+, pnpm.
set -euo pipefail
cd "$(dirname "$0")"

GLIBC="${FLEET_GLIBC:-2.28}"
TARGETS="${FLEET_TARGETS:-x86_64 aarch64}"
TARGET_DIR="${CARGO_TARGET_DIR:-target}"

if [ -z "${CARGO_ZIGBUILD_ZIG_PATH:-}" ] && ! command -v zig >/dev/null; then
  candidate="$(ls "$HOME"/.cache/fleet-board/zig-venv/lib/python*/site-packages/ziglang/zig 2>/dev/null | head -1 || true)"
  [ -n "$candidate" ] || { echo "zig not found. Install one: python3 -m venv ~/.cache/fleet-board/zig-venv && ~/.cache/fleet-board/zig-venv/bin/pip install ziglang" >&2; exit 1; }
  export CARGO_ZIGBUILD_ZIG_PATH="$candidate"
fi
command -v cargo-zigbuild >/dev/null || { echo "cargo-zigbuild not found: cargo install --locked cargo-zigbuild" >&2; exit 1; }

echo "== regenerate TypeScript types and the default UI schema from fleet-api"
cargo test -p fleet-api --test contract ts_bindings_export

echo "== web UI"
pnpm --dir web install --frozen-lockfile
pnpm --dir web test
pnpm --dir web build

mkdir -p dist
for arch in $TARGETS; do
  triple="${arch}-unknown-linux-gnu"
  echo "== fleet-daemon for $triple (glibc $GLIBC)"
  rustup target add "$triple" >/dev/null
  cargo zigbuild --release -p fleet-daemon --target "${triple}.${GLIBC}"
  cp "$TARGET_DIR/$triple/release/fleet-daemon" "dist/fleet-daemon-$arch-linux"
done
ls -l dist
