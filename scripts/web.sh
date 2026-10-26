#!/usr/bin/env bash
# Regenerates the TS types from fleet-api, then tests and builds the web UI into web/dist, which the daemon embeds.
source "$(dirname "$0")/lib.sh"

echo "== regenerate TypeScript types and the default UI schema from fleet-api"
cargo test -p fleet-api --test contract ts_bindings_export

echo "== web UI"
pnpm --dir web install --frozen-lockfile
pnpm --dir web test
pnpm --dir web build
