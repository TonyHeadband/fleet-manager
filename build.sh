#!/usr/bin/env bash
# Builds the web UI, embeds it, and produces fleet-daemon (Linux x86_64 and aarch64) and fleet-gui (x86_64) in ./dist,
# with the installer. Opt-in compile checks: FLEET_CHECK_WINDOWS=1, FLEET_CHECK_MACOS=1.
# Binaries are linked against glibc 2.28 (the floor on Rocky/RHEL 8) through zig, so one build runs everywhere above it.
# Needs: rust + cargo-zigbuild, a zig (`pip install ziglang`), node 20+, pnpm.
set -euo pipefail
cd "$(dirname "$0")"

GLIBC="${FLEET_GLIBC:-2.28}"
TARGETS="${FLEET_TARGETS:-x86_64 aarch64}"
TARGET_DIR="${CARGO_TARGET_DIR:-target}"

if [ -z "${CARGO_ZIGBUILD_ZIG_PATH:-}" ] && ! command -v zig >/dev/null; then
  # ~/.cache/fleet-board is the venv made before the rename; a venv cannot be moved, so look there too.
  candidate="$(ls "$HOME"/.cache/fleet-manager/zig-venv/lib/python*/site-packages/ziglang/zig "$HOME"/.cache/fleet-board/zig-venv/lib/python*/site-packages/ziglang/zig 2>/dev/null | head -1 || true)"
  [ -n "$candidate" ] || { echo "zig not found. Install one: python3 -m venv ~/.cache/fleet-manager/zig-venv && ~/.cache/fleet-manager/zig-venv/bin/pip install ziglang" >&2; exit 1; }
  export CARGO_ZIGBUILD_ZIG_PATH="$candidate"
fi
command -v cargo-zigbuild >/dev/null || { echo "cargo-zigbuild not found: cargo install --locked cargo-zigbuild" >&2; exit 1; }

echo "== rustfmt"
cargo fmt --all --check

echo "== regenerate TypeScript types and the default UI schema from fleet-api"
cargo test -p fleet-api --test contract ts_bindings_export

echo "== web UI"
pnpm --dir web install --frozen-lockfile
pnpm --dir web test
pnpm --dir web build

echo "== fleet-client, fleet-gui and installer tests"
cargo test -p fleet-client -p fleet-gui
bash packaging/test-install.sh

mkdir -p dist
for arch in $TARGETS; do
  triple="${arch}-unknown-linux-gnu"
  echo "== fleet-daemon for $triple (glibc $GLIBC)"
  rustup target add "$triple" >/dev/null
  cargo zigbuild --release -p fleet-daemon --target "${triple}.${GLIBC}"
  cp "$TARGET_DIR/$triple/release/fleet-daemon" "dist/fleet-daemon-$arch-linux"
done

# The app is for the desktop it runs on, so x86_64 only. Same glibc floor as the daemon through zig; if that
# cannot produce a binary, a native build on this machine (whose glibc is the floor on Rocky 8) is the fallback.
if [[ " $TARGETS " == *" x86_64 "* ]]; then
  gui_triple="x86_64-unknown-linux-gnu"
  echo "== fleet-gui for $gui_triple (glibc $GLIBC)"
  if cargo zigbuild --release -p fleet-gui --target "${gui_triple}.${GLIBC}"; then
    cp "$TARGET_DIR/$gui_triple/release/fleet-gui" dist/fleet-gui-x86_64-linux
    echo "fleet-gui: built with zig against glibc $GLIBC"
  else
    echo "fleet-gui: the zig build failed; falling back to a native build" >&2
    cargo build --release -p fleet-gui
    cp "$TARGET_DIR/release/fleet-gui" dist/fleet-gui-x86_64-linux
    echo "fleet-gui: built natively (it needs this machine's glibc or newer)"
  fi
fi

rm -rf dist/packaging
mkdir dist/packaging
cp packaging/install.sh packaging/fleet-gui-launcher packaging/fleet-manager.desktop.in packaging/fleet-manager.svg packaging/config.example.toml dist/packaging/

if [ "${FLEET_CHECK_WINDOWS:-}" = 1 ]; then
  echo "== compile check: fleet-client and fleet-gui for Windows (not a shipped target)"
  rustup target add x86_64-pc-windows-gnu >/dev/null
  cargo check -p fleet-client -p fleet-gui --target x86_64-pc-windows-gnu
fi
if [ "${FLEET_CHECK_MACOS:-}" = 1 ]; then
  echo "== compile check: fleet-client and fleet-gui for macOS (not a supported target yet; failures are reported, not fatal)"
  for t in aarch64-apple-darwin x86_64-apple-darwin; do
    rustup target add "$t" >/dev/null
    cargo check -p fleet-client -p fleet-gui --target "$t" || echo "macOS check failed for $t" >&2
  done
fi

ls -l dist
