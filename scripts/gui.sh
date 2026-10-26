#!/usr/bin/env bash
# Tests fleet-client and fleet-gui, then builds fleet-gui (x86_64 only) into ./dist.
source "$(dirname "$0")/lib.sh"
need_zig

echo "== fleet-client and fleet-gui tests"
cargo test -p fleet-client -p fleet-gui

mkdir -p dist
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
