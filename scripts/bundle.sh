#!/usr/bin/env bash
# Builds the release archive for one OS: dist/fleet-manager-v<version>-{linux-x64.tar.gz,windows-x64.zip}, one top-level
# folder fleet-manager/. Usage: scripts/bundle.sh [--os windows|linux]   (default: the host OS)
# Linux cross-builds through zig against glibc $GLIBC; Windows builds natively with -j2.
# FLEET_BUNDLE_SKIP_BUILD=1 packs existing binaries (or empty stand-ins) without building; FLEET_BUNDLE_OUT overrides dist.
source "$(dirname "$0")/lib.sh"

case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) os=windows ;;
  *) os=linux ;;
esac
if [ "${1:-}" = --os ]; then
  os="${2:-}"
elif [ $# -gt 0 ]; then
  echo "usage: $0 [--os windows|linux]" >&2
  exit 2
fi
case "$os" in windows | linux) ;; *) echo "usage: $0 [--os windows|linux]" >&2; exit 2 ;; esac

version="$(sed -n '/^\[workspace.package\]/,/^\[/{s/^version *= *"\(.*\)"/\1/p;}' Cargo.toml | head -1)"
[ -n "$version" ] || { echo "no workspace version in Cargo.toml" >&2; exit 1; }

programs="fleet-gui fleet-daemon fleet-attach"
if [ "$os" = windows ]; then
  ext=.exe
  bin_dir="$TARGET_DIR/release"
else
  ext=
  triple="x86_64-unknown-linux-gnu"
  bin_dir="$TARGET_DIR/$triple/release"
fi

if [ "${FLEET_BUNDLE_SKIP_BUILD:-}" != 1 ]; then
  if [ "$os" = windows ]; then
    echo "== release build for Windows"
    cargo build --release -j2 -p fleet-gui -p fleet-daemon -p fleet-client --bin fleet-gui --bin fleet-daemon --bin fleet-attach
  else
    need_zig
    echo "== release build for $triple (glibc $GLIBC)"
    cargo zigbuild --release -p fleet-gui -p fleet-daemon -p fleet-client --bin fleet-gui --bin fleet-daemon --bin fleet-attach --target "${triple}.${GLIBC}"
  fi
fi

out="${FLEET_BUNDLE_OUT:-dist}"
stage="$out/fleet-manager"
trap 'rm -rf "$stage" "${tmp_archive:-}"' EXIT
rm -rf "$stage"
mkdir -p "$stage"
for p in $programs; do
  if [ -f "$bin_dir/$p$ext" ]; then
    cp "$bin_dir/$p$ext" "$stage/"
  elif [ "${FLEET_BUNDLE_SKIP_BUILD:-}" = 1 ]; then
    : > "$stage/$p$ext"
  else
    echo "missing $bin_dir/$p$ext" >&2
    exit 1
  fi
done
cp packaging/README.txt LICENSE NOTICE "$stage/"
if [ "$os" = linux ]; then
  cp packaging/fleet-manager.svg "$stage/"
  cp -r packaging/icons "$stage/"
fi

if [ "$os" = windows ]; then
  archive="$out/fleet-manager-v$version-windows-x64.zip"
  tmp_archive="$archive.tmp.zip"
  rm -f "$tmp_archive"
  # Windows 10+ ships bsdtar, which writes a zip with forward slashes; Compress-Archive on PowerShell 5.1 writes backslashes.
  "$(cygpath -u "$SYSTEMROOT")/System32/tar.exe" -a -c -f "$(cygpath -w "$tmp_archive")" -C "$(cygpath -w "$out")" fleet-manager
else
  archive="$out/fleet-manager-v$version-linux-x64.tar.gz"
  tmp_archive="$archive.tmp"
  tar -czf "$tmp_archive" -C "$out" fleet-manager
fi
mv -f "$tmp_archive" "$archive"
echo "$archive"
