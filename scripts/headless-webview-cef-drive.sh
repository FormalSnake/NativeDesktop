#!/usr/bin/env bash
# M2 gate for the Chromium <webview> engine: the SAME probe and the SAME driver
# the WebKitGTK gate uses (examples/webview-probe + scripts/webview-drive.ts),
# with ND_WEBVIEW_ENGINE=chromium. The bar is the pass set WebKitGTK reaches.
# Marker: ND_WEBVIEW2_OK.
#
# Xvfb rather than weston, for the reason scripts/headless-webview-cef.sh gives:
# CEF's windowed embedding is compiled X11-only.
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
export DISPLAY="${ND_CEF_DISPLAY:-:95}"
export GDK_BACKEND=x11
export GSK_RENDERER=cairo
export NATIVE_AUTOMATION=1
export ND_WEBVIEW_ENGINE=chromium
export ND_APP_ID="${ND_APP_ID:-dev.nativedesktop.headlessCefDrive}"
XDG_DATA_HOME="$(mktemp -d)"
export XDG_DATA_HOME
export ND_AUTOMATION_DIALOG_SCRIPT='{"webview.scriptDialog":[{"accepted":true},{"accepted":true},{"accepted":true,"text":"nd-typed"}]}'

Xvfb "$DISPLAY" -screen 0 1400x1000x24 -nolisten tcp >/dev/null 2>&1 &
XVFB_PID=$!
trap 'kill "$XVFB_PID" 2>/dev/null || true; [ -n "${HOST_PID:-}" ] && kill "$HOST_PID" 2>/dev/null; true' EXIT

for _ in $(seq 1 100); do
  xwininfo -root >/dev/null 2>&1 && break
  sleep 0.1
done
xwininfo -root >/dev/null 2>&1 || { echo "FAIL: Xvfb never came up on $DISPLAY"; exit 1; }

LOG=$(mktemp)
ND_WEBVIEW_TRACE=1 ND_SCRIPT=examples/webview-probe/main.tsx ./zig-out/bin/nd-hello >"$LOG" 2>&1 &
HOST_PID=$!

for _ in $(seq 1 1200); do
  grep -q "ND_AUTOMATION_LISTENING" "$LOG" && grep -q "ND_COMMIT_APPLIED" "$LOG" && break
  sleep 0.1
done
grep -q "ND_AUTOMATION_LISTENING" "$LOG" || { echo "FAIL: no automation listener"; tail -60 "$LOG"; exit 1; }
grep -q "ND_COMMIT_APPLIED" "$LOG" || { echo "FAIL: no commit applied"; tail -60 "$LOG"; exit 1; }

for _ in $(seq 1 600); do
  grep -q "ND_WEBVIEW_ENGINE chromium" "$LOG" && break
  sleep 0.1
done
grep -q "ND_WEBVIEW_ENGINE chromium" "$LOG" || { echo "FAIL: the chromium engine did not load"; tail -40 "$LOG"; exit 1; }
if grep -q "falling back to the system engine" "$LOG"; then
  echo "FAIL: a view fell back to the system engine"
  grep -n "ND_WARN" "$LOG" | head -20
  exit 1
fi

SOCK=$(grep -m1 "ND_AUTOMATION_LISTENING" "$LOG" | sed 's/.*path=//')
ND_AUTOMATION_SOCKET="$SOCK" ND_SHOT_PATH="$XDG_RUNTIME_DIR/webview-probe-cef.png" \
  bun scripts/webview-drive.ts >"$XDG_RUNTIME_DIR/drive.log" 2>&1 \
  || { echo "FAIL: driver"; cat "$XDG_RUNTIME_DIR/drive.log"; echo "--- host ---"; tail -60 "$LOG"; exit 1; }
cat "$XDG_RUNTIME_DIR/drive.log"
grep -q "ND_WEBVIEW2_OK" "$XDG_RUNTIME_DIR/drive.log" || { echo "FAIL: driver did not report success"; exit 1; }

kill -TERM "$HOST_PID"; wait "$HOST_PID" 2>/dev/null || true
HOST_PID=""

echo "headless webview drive (chromium): OK"
