#!/usr/bin/env bash
# M1 gate for the Chromium engine on macOS: assembles the dev CEF bundle, runs
# examples/webview-probe/cef-m1.tsx headful with ND_WEBVIEW_ENGINE=chromium,
# and drives it with scripts/cef-m1-drive.ts. Marker: ND_CEF_M1_OK.
#
# Headful, not headless: windowed embedding is the whole point, and the paint
# assertion needs a real compositor. Pass an engine as $1 ("chromium" default,
# "system" runs the identical drive against WKWebView for parity).
set -euo pipefail
cd "$(dirname "$0")/../.."

ENGINE="${1:-chromium}"

if [ "$ENGINE" = "chromium" ]; then
  HOST="$(./scripts/mac/dev-cef-bundle.sh | tail -1)"
else
  HOST="$(./scripts/mac/build-appkit-host.sh | tail -1)"
fi

LOG=$(mktemp)
NATIVE_AUTOMATION=1 ND_WEBVIEW_ENGINE="$ENGINE" ND_WEBVIEW_TRACE=1 \
  ND_SCRIPT=examples/webview-probe/cef-m1.tsx "$HOST" >"$LOG" 2>&1 &
HOST_PID=$!
trap '[ -n "${HOST_PID:-}" ] && kill "$HOST_PID" 2>/dev/null; true' EXIT

for _ in $(seq 1 300); do
  grep -q "ND_AUTOMATION_LISTENING" "$LOG" && break
  sleep 0.1
done
grep -q "ND_AUTOMATION_LISTENING" "$LOG" || { echo "FAIL: no automation listener"; cat "$LOG"; exit 1; }

if [ "$ENGINE" = "chromium" ]; then
  # The engine marker is the host reporting which framework it actually
  # dlopened, so a silent fallback to WKWebView fails here instead of passing
  # the drive on the wrong engine.
  for _ in $(seq 1 300); do
    grep -q "ND_WEBVIEW_ENGINE chromium" "$LOG" && break
    sleep 0.1
  done
  grep -q "ND_WEBVIEW_ENGINE chromium" "$LOG" || {
    echo "FAIL: the chromium engine did not load"; grep -E "ND_WARN|ND_WEBVIEW_ENGINE" "$LOG" | tail -20; exit 1
  }
fi

SOCK=$(grep -m1 "ND_AUTOMATION_LISTENING" "$LOG" | sed 's/.*path=//')
ND_AUTOMATION_SOCKET="$SOCK" bun scripts/cef-m1-drive.ts \
  || { echo "FAIL: driver"; grep -vE "^\[[0-9]+:" "$LOG" | tail -40; exit 1; }

if [ "$ENGINE" = "chromium" ]; then
  # The engine menu is an NSMenu nobody can read back, so the assertion is the
  # host's own trace: the app's items reached Chromium's model, and they did it
  # while on_before_context_menu was still on the stack. A model populated
  # after that call returns is a menu the user sees without them.
  grep -qE "ND_WV cefContextMenu node=[0-9]+ items=3 sync=1" "$LOG" || {
    echo "FAIL: the app's items never reached the engine context menu"
    grep -E "ND_WV cefContextMenu|did not populate synchronously" "$LOG" | tail -5
    exit 1
  }
  ! grep -q "did not populate synchronously" "$LOG" || {
    echo "FAIL: context menu population was not synchronous"; exit 1
  }
  echo "ND_CEF_CTXMENU_OK $(grep -m1 -oE "cefContextMenu node=[0-9]+ items=3 sync=1" "$LOG")"
fi

kill "$HOST_PID"; wait "$HOST_PID" 2>/dev/null || true
HOST_PID=""
echo "cef m1 ($ENGINE): OK"
