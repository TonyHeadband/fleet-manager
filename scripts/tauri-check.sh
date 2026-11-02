#!/usr/bin/env bash
# Runs one manual check of the Tauri app (web/e2e/manual/, not part of CI) in the smoke test's throwaway world:
#   scripts/tauri-check.sh [spec]     default: the Storage widget
# Needs the same tools as tauri-smoke.sh (release binaries, xvfb-run, tauri-driver, WebKitWebDriver).
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
FLEET_E2E_SPECS=${1:-./e2e/manual/storage.check.ts} exec "$root/scripts/tauri-smoke.sh"
