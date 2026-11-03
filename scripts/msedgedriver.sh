#!/usr/bin/env bash
# Downloads the msedgedriver that matches this Windows machine's WebView2 runtime into <dir> and prints its path:
# tauri-driver drives the Tauri app on Windows through it (scripts/tauri-smoke.sh, FLEET_NATIVE_DRIVER). A driver of
# another version refuses the session, so it is fetched for the installed runtime each time. Git Bash only.
# Usage: scripts/msedgedriver.sh <dir>
set -euo pipefail
dir=${1:?usage: scripts/msedgedriver.sh <dir>}

# WebView2's Evergreen runtime, machine-wide or per user.
key='{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
version=
for root in 'HKLM\SOFTWARE\WOW6432Node' 'HKCU\SOFTWARE'; do
  version=$(reg query "$root\\Microsoft\\EdgeUpdate\\Clients\\$key" //v pv 2> /dev/null | awk '/pv/ { print $3 }') || true
  [ -z "$version" ] || break
done
[ -n "$version" ] || { echo "no WebView2 runtime found" >&2; exit 1; }

mkdir -p "$dir"
zip=$dir/edgedriver-$version.zip
curl -fsSL -o "$zip" "https://msedgedriver.microsoft.com/$version/edgedriver_win64.zip"
unzip -oq "$zip" msedgedriver.exe -d "$dir"
rm -f "$zip"
cygpath -m "$dir/msedgedriver.exe"
