#!/usr/bin/env bash
# Produces fleet-daemon and fleet-attach (Linux x86_64 and aarch64) and fleet-gui (x86_64) in ./dist, with the release archive. The web UI
# is not built: the daemon serves its placeholder page until remote access brings the UI back.
# Opt-in compile checks: FLEET_CHECK_WINDOWS=1, FLEET_CHECK_MACOS=1.
# Binaries are linked against glibc 2.28 (the floor on Rocky/RHEL 8) through zig, so one build runs everywhere above it.
# Needs: rust + cargo-zigbuild and a zig (pip install ziglang).
source "$(dirname "$0")/scripts/lib.sh"
need_zig

echo "== rustfmt"
cargo fmt --all --check

scripts/daemon.sh
scripts/gui.sh
scripts/bundle.sh --os linux

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
