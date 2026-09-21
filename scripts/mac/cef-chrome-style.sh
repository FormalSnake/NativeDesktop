#!/usr/bin/env bash
# Chrome style gate for the Chromium engine on macOS. Assembles the dev CEF
# bundle, runs examples/webview-probe/cef-chrome.tsx headful with
# ND_CEF_STYLE=chrome and the probe extension loaded, and drives it with
# scripts/mac/cef-chrome-drive.ts. Marker: ND_CEF_CHROME_OK.
#
# Headful and with a remote debugging port: the extension proof is a
# chrome-extension:// service_worker target in /json/list, which only Chromium's
# own extension runtime can produce, and the paint assertion needs a real
# compositor.
#
# Two runs, because quitting with DevTools docked still crashes Chromium's
# shutdown: the first covers the whole drive including the dock and is killed
# outright, the second skips the dock and asserts the host exits cleanly.
set -euo pipefail
cd "$(dirname "$0")/../.."

PORT="${ND_CEF_DEBUG_PORT:-9334}"
EXTENSION="$(pwd)/examples/webview-probe/chrome-style-ext"
HOST="$(./scripts/mac/dev-cef-bundle.sh | tail -1)"
HOST_PID=""
# Guarded, not `&& … ; true`: the trap runs under `set -e`, so a test that is
# merely false ends the trap there and that status becomes the script's own.
trap 'if [ -n "${HOST_PID:-}" ]; then kill -9 "$HOST_PID" 2>/dev/null || true; fi' EXIT

launch() {
  LOG=$(mktemp)
  NATIVE_AUTOMATION=1 ND_WEBVIEW_ENGINE=chromium ND_CEF_STYLE=chrome ND_WEBVIEW_TRACE=1 \
    ND_SCRIPT=examples/webview-probe/cef-chrome.tsx "$HOST" \
    "--load-extension=$EXTENSION" "--remote-debugging-port=$PORT" --remote-allow-origins='*' >"$LOG" 2>&1 &
  HOST_PID=$!
  for _ in $(seq 1 400); do
    grep -q "ND_AUTOMATION_LISTENING" "$LOG" && break
    sleep 0.1
  done
  grep -q "ND_AUTOMATION_LISTENING" "$LOG" || { echo "FAIL: no automation listener"; cat "$LOG"; exit 1; }
  for _ in $(seq 1 400); do
    grep -q "ND_WEBVIEW_ENGINE chromium" "$LOG" && break
    sleep 0.1
  done
  grep -q "ND_WEBVIEW_ENGINE chromium" "$LOG" || {
    echo "FAIL: the chromium engine did not load"; grep -E "ND_WARN|ND_WEBVIEW_ENGINE" "$LOG" | tail -20; exit 1
  }
  SOCK=$(grep -m1 "ND_AUTOMATION_LISTENING" "$LOG" | sed 's/.*path=//')
}

launch
ND_AUTOMATION_SOCKET="$SOCK" ND_HOST_PID="$HOST_PID" ND_CEF_DEBUG_PORT="$PORT" ND_HOST_LOG="$LOG" \
  ND_APP_HOST_LOG="$LOG" \
  bun scripts/mac/cef-chrome-drive.ts \
  || { echo "FAIL: driver"; grep -vE "^\[[0-9]+:" "$LOG" | tail -40; exit 1; }

# The app's context-menu items have to reach Chromium's own model while
# on_before_context_menu is still on the stack; the menu itself is an NSMenu
# nobody can read back, so the host's trace is the assertion.
grep -qE "ND_WV cef node=[0-9]+ cefContextMenu node=[0-9]+ items=3 sync=1" "$LOG" \
  || grep -qE "cefContextMenu node=[0-9]+ items=3 sync=1" "$LOG" || {
  echo "FAIL: the app's items never reached the engine context menu"
  grep -E "cefContextMenu|did not populate synchronously" "$LOG" | tail -5
  exit 1
}
echo "ND_CEF_CHROME_CTXMENU_OK"
kill -9 "$HOST_PID" 2>/dev/null || true
wait "$HOST_PID" 2>/dev/null || true

# Second run: everything but the DevTools dock, then a real quit.
launch
ND_AUTOMATION_SOCKET="$SOCK" ND_HOST_PID="$HOST_PID" ND_CEF_DEBUG_PORT="$PORT" \
  ND_CEF_CHROME_SKIP_DEVTOOLS=1 bun scripts/mac/cef-chrome-drive.ts >/dev/null \
  || { echo "FAIL: driver (no devtools)"; grep -vE "^\[[0-9]+:" "$LOG" | tail -40; exit 1; }

kill "$HOST_PID"
for _ in $(seq 1 200); do kill -0 "$HOST_PID" 2>/dev/null || break; sleep 0.1; done
if kill -0 "$HOST_PID" 2>/dev/null; then
  echo "FAIL: the host did not exit"; kill -9 "$HOST_PID"; exit 1
fi
set +e
wait "$HOST_PID"
STATUS=$?
set -e
[ "$STATUS" -eq 0 ] || { echo "FAIL: the host exited $STATUS instead of cleanly"; exit 1; }
HOST_PID=""
echo "cef chrome style: OK"
