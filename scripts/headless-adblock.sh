#!/usr/bin/env bash
# Content-blocking gate for the Chromium engine on the GTK host: runs
# examples/adblock-probe on Xvfb under Chrome style and checks every phase
# with scripts/adblock-verdict.ts. Marker: ND_ADBLOCK_OK.
set -euo pipefail
cd "$(dirname "$0")/.."

: "${ND_CEF_DIST:=$HOME/.cache/nativedesktop/cef/151.3.23-linux64}"
[ -f "$ND_CEF_DIST/Release/libcef.so" ] || { echo "SKIP: no CEF distribution at $ND_CEF_DIST"; exit 0; }
ln -sfn "$ND_CEF_DIST"/Resources/* "$ND_CEF_DIST/Release/"
export ND_CEF_ROOT="$ND_CEF_DIST/Release"
if [ -n "${ND_CEF_LD_LIBRARY_PATH:-}" ]; then
  export LD_LIBRARY_PATH="$ND_CEF_LD_LIBRARY_PATH${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi

RUN_DIR="$(mktemp -d)"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$RUN_DIR/run}"
mkdir -p "$XDG_RUNTIME_DIR"
export XDG_DATA_HOME="$RUN_DIR/data"
export ND_CEF_CACHE="${ND_CEF_CACHE:-$RUN_DIR/cef}"
export DISPLAY="${ND_CEF_DISPLAY:-:97}"
export GDK_BACKEND=x11
export GSK_RENDERER=cairo
export NATIVE_AUTOMATION=1
export ND_WEBVIEW_ENGINE=chromium
export ND_CEF_STYLE="${ND_CEF_STYLE:-chrome}"
export ND_APP_ID="${ND_APP_ID:-dev.nativedesktop.headlessAdblock}"
LOG="${ND_ADBLOCK_LOG:-$RUN_DIR/host.log}"

XVFB_PID=""
HOST_PID=""
if [ -z "${ND_CEF_KEEP_DISPLAY:-}" ]; then
  Xvfb "$DISPLAY" -screen 0 1280x900x24 -nolisten tcp >/dev/null 2>&1 &
  XVFB_PID=$!
fi
trap '[ -n "$XVFB_PID" ] && kill "$XVFB_PID" 2>/dev/null; [ -n "$HOST_PID" ] && kill -9 "$HOST_PID" 2>/dev/null; rm -rf "$RUN_DIR"; true' EXIT
for _ in $(seq 1 100); do
  xwininfo -root >/dev/null 2>&1 && break
  sleep 0.1
done

ND_WEBVIEW_TRACE=1 ND_SCRIPT=examples/adblock-probe/main.tsx ./zig-out/bin/nd-hello >"$LOG" 2>&1 &
HOST_PID=$!
for _ in $(seq 1 900); do
  grep -q "ND_ADBLOCK_PROBE_DONE" "$LOG" && break
  kill -0 "$HOST_PID" 2>/dev/null || break
  sleep 0.1
done
grep -q "ND_ADBLOCK_PROBE_DONE" "$LOG" || {
  echo "FAIL: the probe did not finish"
  grep -E "ND_ADBLOCK|ND_WARN|ND_WEBVIEW_ENGINE" "$LOG" | tail -20
  exit 1
}
bun scripts/adblock-verdict.ts "$LOG"
