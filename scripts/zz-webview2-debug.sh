#!/usr/bin/env bash
# TEMPORARY (windows-parity-a): why WebView2 never starts on the Windows runner. Remove before merge.
set -u
app=$(cygpath -w "$PWD/target/release/fleet-gui-tauri.exe")
echo "== whoami: $(whoami)  session: $(powershell -NoProfile -c '(Get-Process -Id $PID).SessionId')"
reg query 'HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}' //v pv
reg query 'HKLM\SOFTWARE\Policies\Microsoft\Edge' 2>&1 | head -20
reg query 'HKLM\SOFTWARE\Policies\Microsoft\Edge\WebView2' 2>&1 | head -20
reg query 'HKCU\SOFTWARE\Policies\Microsoft\Edge' //s 2>&1 | head -20
env | grep -i webview2
for mode in plain override; do
  echo "== mode $mode"
  data="$LOCALAPPDATA/com.proxima-station.fleet-manager/EBWebView"
  rm -rf "$data"
  home=$(mktemp -d)
  (
    [ "$mode" = override ] && export USERPROFILE=$(cygpath -w "$home")
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9333 --enable-logging --v=0" "$app" > "$home/app.out" 2>&1 &
    echo $! > "$home/pid"
  )
  ok=no
  for _ in $(seq 120); do curl -fs http://127.0.0.1:9333/json/version > "$home/v.json" 2>/dev/null && { ok=yes; break; }; sleep 0.5; done
  echo "devtools reachable: $ok"; cat "$home/v.json" 2>/dev/null; echo
  powershell -NoProfile -c "Get-CimInstance Win32_Process -Filter \"Name='msedgewebview2.exe'\" | ForEach-Object { \$_.ProcessId.ToString() + ' ' + \$_.CommandLine.Substring(0, [Math]::Min(3000, \$_.CommandLine.Length)) }" | grep -v -- "--type=" | head -5
  netstat -ano | grep LISTENING | grep -E ":93|:9222" | head
  echo "-- app.out"; tail -20 "$home/app.out"
  echo "-- data dir"; ls -la "$data" 2>&1 | head; find "$LOCALAPPDATA/com.proxima-station.fleet-manager" -maxdepth 3 -name '*.log' 2>/dev/null | while read f; do echo "--- $f"; tail -30 "$f"; done
  # Only this app's own process tree: never other apps' WebView2.
  for p in $(powershell -NoProfile -c "Get-CimInstance Win32_Process -Filter \"Name='fleet-gui-tauri.exe'\" | Where-Object { \$_.ExecutablePath -eq '$app' } | % ProcessId"); do taskkill //F //T //PID "$p" > /dev/null 2>&1; done; sleep 2
done
true
