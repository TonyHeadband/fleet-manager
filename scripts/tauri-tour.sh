#!/usr/bin/env bash
# The Windows/Linux tour: the smoke test's throwaway world with the demo's whole timeline played, then
# web/e2e/manual/tour.check.ts saves a screenshot of every view and tab into <shots-dir>, to lay the two OSes side by
# side. Runs where tauri-smoke.sh runs (Git Bash on Windows with FLEET_NATIVE_DRIVER; Linux, FLEET_SHOW=1 for a desktop
# such as WSLg). Usage: scripts/tauri-tour.sh <shots-dir>
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
shots=${1:?usage: scripts/tauri-tour.sh <shots-dir>}
mkdir -p "$shots"
FLEET_SHOTS=$(cd "$shots" && pwd)
case "$(uname -s)" in MINGW* | MSYS* | CYGWIN*) FLEET_SHOTS=$(cygpath -m "$FLEET_SHOTS") ;; esac
export FLEET_SHOTS FLEET_WORLD=full FLEET_E2E_SPECS=./e2e/manual/tour.check.ts
exec "$root/scripts/tauri-smoke.sh"
