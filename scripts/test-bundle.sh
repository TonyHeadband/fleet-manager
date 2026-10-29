#!/usr/bin/env bash
# Tests scripts/bundle.sh for the host OS without building: FLEET_BUNDLE_SKIP_BUILD=1 packs existing target binaries
# or empty stand-ins. Asserts the archive's file list and that its name carries the Cargo.toml version.
source "$(dirname "$0")/lib.sh"

version="$(sed -n '/^\[workspace.package\]/,/^\[/{s/^version *= *"\(.*\)"/\1/p;}' Cargo.toml | head -1)"
[ -n "$version" ] || { echo "FAIL: no workspace version in Cargo.toml" >&2; exit 1; }

case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*)
    os=windows
    archive="fleet-manager-v$version-windows-x64.zip"
    expected="LICENSE NOTICE README.txt fleet-attach.exe fleet-daemon.exe fleet-gui.exe"
    ;;
  *)
    os=linux
    archive="fleet-manager-v$version-linux-x64.tar.gz"
    icons="$(cd packaging && find icons -type f | sort | sed 's/^/ /' | tr -d '\n')"
    expected="LICENSE NOTICE README.txt fleet-attach fleet-daemon fleet-gui fleet-manager.svg$icons"
    ;;
esac
expected="$(printf '%s\n' $expected | sed 's|^|fleet-manager/|' | sort)"

out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT
printed="$(FLEET_BUNDLE_SKIP_BUILD=1 FLEET_BUNDLE_OUT="$out" bash scripts/bundle.sh --os "$os" | tail -1)"

fail=0
[ "$(basename "$printed")" = "$archive" ] || { echo "FAIL: printed '$printed', expected name $archive" >&2; fail=1; }
[ -f "$out/$archive" ] || { echo "FAIL: $out/$archive was not written" >&2; exit 1; }
if [ "$os" = windows ]; then
  actual="$(unzip -Z1 "$out/$archive" | sed 's,[\],/,g' | grep -v '/$' | sort)"
else
  actual="$(tar -tzf "$out/$archive" | grep -v '/$' | sort)"
fi
if [ "$actual" != "$expected" ]; then
  echo "FAIL: archive file list differs" >&2
  diff <(echo "$expected") <(echo "$actual") >&2 || true
  fail=1
fi
[ "$fail" = 0 ] && echo "bundle test ($os): ok"
exit "$fail"
