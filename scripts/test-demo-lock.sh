#!/usr/bin/env bash
# Exercises the demo.sh fingerprint lock in a throwaway repo so the real demo/demo.lock is never touched.
set -euo pipefail
cd "$(dirname "$0")/.."
src="$PWD"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

mkdir -p "$tmp/scripts"
cp "$src/scripts/demo.sh" "$tmp/scripts/"
[ -d "$src/demo" ] && cp -r "$src/demo" "$tmp/demo"
rm -f "$tmp/demo/demo.lock"
cd "$tmp"
git init -q
git config user.name test
git config user.email test@example.invalid
git add -A
git commit -qm init

want="demo media is stale, run scripts/demo.sh and commit docs/assets/ and demo/demo.lock"
fail() { echo "FAIL: $*" >&2; exit 1; }

# $1 = expected exit code; the stale message is checked whenever it is 1.
expect_check() {
  local want_rc="$1" label="$2" out rc=0
  out="$(bash scripts/demo.sh --check 2>&1)" || rc=$?
  [ "$rc" -eq "$want_rc" ] || fail "$label: exit $rc, want $want_rc ($out)"
  if [ "$want_rc" -eq 1 ] && [ "$out" != "$want" ]; then fail "$label: wrong message: $out"; fi
}

expect_check 1 "missing lock"

bash scripts/demo.sh --lock
[ "$(wc -l < demo/demo.lock)" -eq 2 ] || fail "lock must have two lines"
head -1 demo/demo.lock | grep -qx '# regenerate with scripts/demo.sh; do not edit' || fail "lock header"
tail -1 demo/demo.lock | grep -Eqx 'sha256:[0-9a-f]{64}' || fail "lock hash format"
git add demo/demo.lock
git commit -qm lock
expect_check 0 "fresh lock"

echo untracked > demo/untracked.txt
expect_check 0 "untracked file under demo/"
rm demo/untracked.txt

scenario="$(git ls-files demo/scenarios | head -1)"
[ -n "$scenario" ] || fail "no scenario file in demo/scenarios"
echo "# edit" >> "$scenario"
git commit -qam "edit scenario"
expect_check 1 "committed scenario edit"

bash scripts/demo.sh --lock
expect_check 0 "relocked"

rm demo/demo.lock
expect_check 1 "lock removed again"

echo "ok"
