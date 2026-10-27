#!/usr/bin/env bash
# Exercises the demo.sh fingerprint lock in a throwaway repo so the real demo/demo.lock is never touched.
set -euo pipefail
cd "$(dirname "$0")/.."
src="$PWD"

fail() { echo "FAIL: $*" >&2; exit 1; }

# A fingerprinted input outside the docs-media paths could go stale without the gate running --check.
read -ra inputs <<< "$(sed -nE 's/^COMMON_INPUTS=\((.*)\)$/\1/p' scripts/demo.sh) demo/shots"
[ ${#inputs[@]} -gt 1 ] || fail "no COMMON_INPUTS in scripts/demo.sh"
gate_paths="$(awk '/^  docs-media:/ { s = 1; next } s && /^  [^ ]/ { exit } s && /^    paths:/ { print }' .fleet/workflow.yml \
  | sed -E 's/^ *paths: *\[(.*)\] *$/\1/; s/, */ /g')"
read -ra globs <<< "$gate_paths"
[ ${#globs[@]} -gt 0 ] || fail "no paths for docs-media in .fleet/workflow.yml"
for input in "${inputs[@]}"; do
  covered=0
  for glob in "${globs[@]}"; do
    prefix="${glob%/\*\*}"
    if [ "$input" = "$glob" ] || { [ "$prefix" != "$glob" ] && [[ "$input" == "$prefix" || "$input" == "$prefix"/* ]]; }; then
      covered=1
      break
    fi
  done
  [ "$covered" -eq 1 ] || fail "fingerprinted input $input is not covered by the docs-media paths"
done

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

# $1 = expected exit code; the stale message is checked whenever it is 1.
expect_check() {
  local want_rc="$1" label="$2" out rc=0
  out="$(bash scripts/demo.sh --check 2>&1)" || rc=$?
  [ "$rc" -eq "$want_rc" ] || fail "$label: exit $rc, want $want_rc ($out)"
  if [ "$want_rc" -eq 1 ] && [ "$out" != "$want" ]; then fail "$label: wrong message: $out"; fi
}

expect_check 1 "missing lock"

bash scripts/demo.sh --lock
[ "$(wc -l < demo/demo.lock)" -eq 2 ] || fail "lock must have a header and one line per shot"
head -1 demo/demo.lock | grep -qx '# regenerate with scripts/demo.sh; do not edit' || fail "lock header"
tail -1 demo/demo.lock | grep -Eqx 'pie sha256:[0-9a-f]{64}' || fail "lock line format"
git add demo/demo.lock
git commit -qm lock
expect_check 0 "fresh lock"

echo untracked > demo/untracked.txt
expect_check 0 "untracked file under demo/"
rm demo/untracked.txt

scenario="$(git ls-files demo/shots/pie | grep /scenarios/ | head -1)"
[ -n "$scenario" ] || fail "no scenario file in demo/shots/pie"
echo "# edit" >> "$scenario"
git commit -qam "edit scenario"
expect_check 1 "stale pie fails"

bash scripts/demo.sh --lock
expect_check 0 "relocked"

# A second official shot: its lock-inputs names a file outside its folder.
mkdir demo/shots/extra
cp demo/shots/pie/shot.toml demo/shots/extra/shot.toml
echo "scripts/extra-input.txt" > demo/shots/extra/lock-inputs
echo one > scripts/extra-input.txt
git add demo scripts/extra-input.txt
git commit -qm "add extra"
bash scripts/demo.sh --lock
[ "$(wc -l < demo/demo.lock)" -eq 3 ] || fail "lock must have a line per official shot"
grep -Eq '^extra sha256:[0-9a-f]{64}$' demo/demo.lock || fail "no line for extra"
git add demo/demo.lock
git commit -qm "lock extra"
expect_check 0 "extra fresh"

# Only extra's line may change.
pie_line="$(grep '^pie ' demo/demo.lock)"
echo "# edit" >> demo/shots/extra/shot.toml
git commit -qam "edit extra"
err="$(bash scripts/demo.sh --check 2>&1 >/dev/null)" || fail "stale short must not fail --check"
grep -q 'warning: demo shot extra is stale, run scripts/demo.sh --shot demo/shots/extra' <<< "$err" || fail "stale short warns: $err"
bash scripts/demo.sh --lock --shot demo/shots/extra
[ "$(grep '^pie ' demo/demo.lock)" = "$pie_line" ] || fail "--lock --shot rewrote the pie's line"
expect_check 0 "extra relocked"
git add demo/demo.lock
git commit -qm "relock extra"

echo two > scripts/extra-input.txt
git commit -qam "edit extra input"
err="$(bash scripts/demo.sh --check 2>&1 >/dev/null)" || fail "lock-inputs change must only warn"
grep -q 'demo shot extra is stale' <<< "$err" || fail "lock-inputs counts: extra not stale"
! grep -q 'demo media is stale' <<< "$err" || fail "lock-inputs counts: pie went stale"
bash scripts/demo.sh --lock --shot demo/shots/extra
git add demo/demo.lock
git commit -qm "relock extra again"

# A shot elsewhere is not the lock's business and must say where its media goes before anything is built.
before="$(sha256sum demo/demo.lock)"
rc=0
err="$(bash scripts/demo.sh --shot "$tmp/elsewhere" 2>&1)" || rc=$?
[ "$rc" -eq 2 ] || fail "outside shot without --out: exit $rc, want 2 ($err)"
[ "$err" = "--out is required for a shot outside demo/shots" ] || fail "outside shot message: $err"
[ "$(sha256sum demo/demo.lock)" = "$before" ] || fail "outside shot touched demo/demo.lock"

rc=0
err="$(bash scripts/demo.sh --formats , 2>&1)" || rc=$?
[ "$rc" -eq 2 ] || fail "empty --formats: exit $rc, want 2 ($err)"

git rm -rq demo/shots/extra
rm demo/demo.lock
expect_check 1 "lock removed again"

echo "ok"
