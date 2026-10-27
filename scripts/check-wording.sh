#!/usr/bin/env bash
# The unit of work is a "task". Fails on any "project" or "projects" left in the shipped code, UI text, docs and demo.
# Out of scope: SQL migrations, generated bindings, and the records under docs/superpowers, ROADMAP.md and
# docs/concept.md. A line that must keep the word (a table name, the refused old config key) carries `wording: allow`.
set -euo pipefail
cd "$(dirname "$0")/.."

hits=$(grep -rniwE 'projects?' \
    --exclude-dir=migrations --exclude-dir=generated --exclude-dir=node_modules --exclude-dir=target \
    --exclude-dir=dist --exclude=check-wording.sh --exclude='LICENSE*.txt' \
    crates web/src packaging scripts demo README.md docs/usage.md docs/ui.md docs/demo.md \
  | grep -v 'wording: allow' || true)

if [ -n "$hits" ]; then
  echo "$hits"
  echo "check-wording: say \"task\", not \"project\" (or mark the line 'wording: allow')" >&2
  exit 1
fi
