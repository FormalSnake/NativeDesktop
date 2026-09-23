#!/usr/bin/env bash
# Zoom gate for the Chromium engine in Chrome style on macOS: assembles the
# dev CEF bundle, runs examples/webview-probe/cef-zoom.tsx and drives it with
# scripts/cef-zoom-drive.ts, the same drive the Linux gate runs. Proves that
# setZoom and the zoom chords reach the app as `zoomChanged` and that
# Chromium's zoom bubble is closed before it is ever placed over the page.
# The chords go through app.cursor and real key events: hold the mac CEF gate
# lock and give the run its own ND_CEF_CACHE. Marker: ND_CEF_ZOOM_OK.
set -euo pipefail
cd "$(dirname "$0")/../.."

PORT="${ND_CEF_DEBUG_PORT:-9336}"
HOST="$(./scripts/mac/dev-cef-bundle.sh | tail -1)"
HOST_PID=""
trap 'if [ -n "${HOST_PID:-}" ]; then kill -9 "$HOST_PID" 2>/dev/null || true; fi' EXIT

LOG=$(mktemp)
NATIVE_AUTOMATION=1 ND_WEBVIEW_ENGINE=chromium ND_CEF_STYLE=chrome ND_WEBVIEW_TRACE=1 \
  ND_SCRIPT=examples/webview-probe/cef-zoom.tsx "$HOST" \
  "--load-extension=$(pwd)/scripts/fixtures/password-filler" "--remote-debugging-port=$PORT" --remote-allow-origins='*' >"$LOG" 2>&1 &
HOST_PID=$!
for _ in $(seq 1 400); do
  grep -q "ND_AUTOMATION_LISTENING" "$LOG" && grep -q "ND_COMMIT_APPLIED" "$LOG" && break
  sleep 0.1
done
grep -q "ND_AUTOMATION_LISTENING" "$LOG" || { echo "FAIL: no automation listener"; tail -40 "$LOG"; exit 1; }
SOCK=$(grep -m1 "ND_AUTOMATION_LISTENING" "$LOG" | sed 's/.*path=//')

ND_AUTOMATION_SOCKET="$SOCK" ND_HOST_PID="$HOST_PID" ND_HOST_LOG="$LOG" ND_CEF_DEBUG_PORT="$PORT" \
  bun scripts/cef-zoom-drive.ts || { echo "FAIL: zoom drive"; grep -E "zoom|surface" "$LOG" | tail -30; exit 1; }
echo "mac cef zoom: OK"
