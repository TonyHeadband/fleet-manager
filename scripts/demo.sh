#!/usr/bin/env bash
# Demo media pipeline. --check is the CI gate: it only compares the committed fingerprint, never records.
set -euo pipefail
cd "$(dirname "$0")/.."

# Tracked inputs that change what the demo shows. Keep in sync with the docs-media paths in .fleet/workflow.yml.
DEMO_INPUTS=(crates/fleet-gui crates/fleet-client crates/fleet-api crates/fleet-demo crates/fleet-daemon/src/status.rs fixtures/hooks demo/scenarios demo/timeline.toml demo/bin scripts/demo.sh Cargo.lock)
LOCK=demo/demo.lock

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

case "${1:-}" in
  --check) check_lock ;;
  --lock) write_lock ;;
  "") echo "recording is not implemented yet" >&2; exit 2 ;;
  *) echo "usage: scripts/demo.sh [--check|--lock]" >&2; exit 2 ;;
esac
