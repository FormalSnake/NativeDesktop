#!/usr/bin/env bash
# "Page Unresponsive" gate for the Chromium engine on Linux: runs
# examples/webview-probe/cef-hang.tsx under Xvfb and drives it with
# scripts/cef-hang-drive.ts. A page waiting on a stalled navigation, a new
# renderer or its own alert is never asked about; a spinning page is, and Wait
# and Exit page work. Marker: ND_CEF_HANG_OK.
set -euo pipefail
cd "$(dirname "$0")/.."

: "${ND_CEF_DIST:=$HOME/.cache/nativedesktop/cef/151.3.23-linux64}"
[ -f "$ND_CEF_DIST/Release/libcef.so" ] || { echo "SKIP: no CEF distribution at $ND_CEF_DIST"; exit 0; }
ln -sfn "$ND_CEF_DIST"/Resources/* "$ND_CEF_DIST/Release/"
export ND_CEF_ROOT="$ND_CEF_DIST/Release"

if [ -n "${ND_CEF_LD_LIBRARY_PATH:-}" ]; then
  export LD_LIBRARY_PATH="$ND_CEF_LD_LIBRARY_PATH${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
export DISPLAY="${ND_CEF_DISPLAY:-:97}"
export GDK_BACKEND=x11
export GSK_RENDERER=cairo
export NATIVE_AUTOMATION=1
export ND_WEBVIEW_ENGINE=chromium
export ND_CEF_STYLE=chrome
export ND_APP_ID="${ND_APP_ID:-dev.nativedesktop.headlessCefHang}"
XDG_DATA_HOME="$(mktemp -d)"
export XDG_DATA_HOME

XVFB_PID=""
if [ -z "${ND_CEF_KEEP_DISPLAY:-}" ]; then
  Xvfb "$DISPLAY" -screen 0 1280x900x24 -nolisten tcp >/dev/null 2>&1 &
  XVFB_PID=$!
fi
trap '[ -n "$XVFB_PID" ] && kill "$XVFB_PID" 2>/dev/null; [ -n "${HOST_PID:-}" ] && kill "$HOST_PID" 2>/dev/null; true' EXIT

for _ in $(seq 1 100); do
  xwininfo -root >/dev/null 2>&1 && break
  sleep 0.1
done
xwininfo -root >/dev/null 2>&1 || { echo "FAIL: no X server on $DISPLAY"; exit 1; }

LOG="$XDG_RUNTIME_DIR/host-cef-hang.log"
ND_WEBVIEW_TRACE=1 BUN_OPTIONS="$(bun scripts/bun-options.ts examples/webview-probe/cef-hang.tsx)" ND_SCRIPT=examples/webview-probe/cef-hang.tsx ./zig-out/bin/nd-hello \
  >"$LOG" 2>&1 &
HOST_PID=$!
for _ in $(seq 1 900); do
  grep -q "ND_AUTOMATION_LISTENING" "$LOG" && grep -q "ND_COMMIT_APPLIED" "$LOG" && break
  sleep 0.1
done
grep -q "ND_AUTOMATION_LISTENING" "$LOG" || { echo "FAIL: no automation listener"; tail -40 "$LOG"; exit 1; }
SOCK=$(grep -m1 "ND_AUTOMATION_LISTENING" "$LOG" | sed 's/.*path=//')

DRIVE_LOG="$XDG_RUNTIME_DIR/cef-hang-drive.log"
ND_AUTOMATION_SOCKET="$SOCK" ND_HOST_PID="$HOST_PID" ND_HOST_LOG="$LOG" \
  bun scripts/cef-hang-drive.ts >"$DRIVE_LOG" 2>&1 || true
cat "$DRIVE_LOG"
grep -q "ND_CEF_HANG_OK" "$DRIVE_LOG" || {
  echo "FAIL: hang drive"
  grep -E "pageUnresponsive|pageDialog|renderProcessGone|ND_WARN" "$LOG" | tail -30
  exit 1
}
echo "headless cef hang: OK (ND_CEF_HANG_OK)"
