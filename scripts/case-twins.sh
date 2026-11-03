#!/usr/bin/env bash
# Fails on tracked files whose names differ only by case. Windows and macOS filesystems ignore case, so such a pair
# checks out as one file there, and an import like `./widget` finds `Widget.tsx` as readily as `widget.ts`. Linux
# CI is where twins get written, so both CIs run this. Two checks: whole paths, and script modules with their last
# extension (.ts .tsx .js .jsx .mts .mjs) removed, since an extensionless import resolves to either.
# Usage: scripts/case-twins.sh [repo-dir]   Prints one line per clash and exits 1, or nothing and exits 0.
set -euo pipefail
cd "${1:-$(dirname "$0")/..}"

twins() {
  # Input: one "key<TAB>path" per line. Output: the paths sharing a lowercased key, one group per line.
  awk -F'\t' '{ k = tolower($1); n[k]++; g[k] = g[k] (g[k] ? " " : "") $2 }
    END { for (k in n) if (n[k] > 1) print g[k] }' | sort
}

files=$(git ls-files)
clashes=$(
  {
    printf '%s\n' "$files" | awk '{ print $0 "\t" $0 }' | twins
    printf '%s\n' "$files" | grep -E '\.(ts|tsx|js|jsx|mts|mjs)$' | sed -E 's/^(.*)\.[a-z]+$/\1\t&/' | twins
  } | sort -u
)
[ -z "$clashes" ] || { printf '%s\n' "$clashes"; exit 1; }
