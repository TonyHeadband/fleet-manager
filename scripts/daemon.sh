#!/usr/bin/env bash
# Tests and cross-builds fleet-daemon into ./dist. It embeds whatever web/dist holds (or the placeholder page).
source "$(dirname "$0")/lib.sh"
need_zig

echo "== fleet-daemon tests"
cargo test -p fleet-daemon

mkdir -p dist
for arch in $TARGETS; do
  triple="${arch}-unknown-linux-gnu"
  echo "== fleet-daemon for $triple (glibc $GLIBC)"
  rustup target add "$triple" >/dev/null
  cargo zigbuild --release -p fleet-daemon --target "${triple}.${GLIBC}"
  cp "$TARGET_DIR/$triple/release/fleet-daemon" "dist/fleet-daemon-$arch-linux"
done
