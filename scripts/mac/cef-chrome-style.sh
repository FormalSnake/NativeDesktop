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
SECOND_PID=""
# shellcheck source=scripts/mac/cef-gate-lock.sh
. scripts/mac/cef-gate-lock.sh
# Guarded, not `&& … ; true`: the trap runs under `set -e`, so a test that is
# merely false ends the trap there and that status becomes the script's own.
cleanup() {
  if [ -n "${HOST_PID:-}" ]; then kill -9 "$HOST_PID" 2>/dev/null || true; fi
  if [ -n "${SECOND_PID:-}" ]; then kill -9 "$SECOND_PID" 2>/dev/null || true; fi
  cef_gate_unlock
}
trap cleanup EXIT
cef_gate_lock
CACHE_DIR="${ND_CEF_CACHE:-$RUN_DIR/cef}"

launch() {
  LOG="$(mktemp "$RUN_DIR/host.XXXXXX")"
  NATIVE_AUTOMATION=1 ND_WEBVIEW_ENGINE=chromium ND_CEF_STYLE=chrome ND_WEBVIEW_TRACE=1 \
    ND_CEF_CACHE="$CACHE_DIR" ND_SCRIPT=examples/webview-probe/cef-chrome.tsx "$HOST" \
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
ND_AUTOMATION_SOCKET="$SOCK" ND_HOST_PID="$HOST_PID" ND_CEF_DEBUG_PORT="$PORT" ND_HOST_LOG="$LOG" \
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

# The relaunch leg. A host killed mid-session comes back on the same cache, and
# a second host is then started against the one holding it. Chrome answers both
# with a startup browser of its own, and the second with "Restore pages?" on top
# when the profile's last exit was a crash; neither may reach the screen.
port_free() {
  for _ in $(seq 1 100); do
    curl -s --max-time 1 "http://127.0.0.1:$PORT/json/version" >/dev/null 2>&1 || return 0
    sleep 0.2
  done
  return 1
}
relaunch_drive() {
  ND_RELAUNCH_STEP="$1" ND_HOST_PID="$HOST_PID" ND_CEF_DEBUG_PORT="$PORT" ND_HOST_LOG="$LOG" ND_RELAUNCH_SHOT_DIR="$SHOT_DIR" \
    ND_AUTOMATION_SOCKET="$SOCK" bun scripts/cef-relaunch-drive.ts || RELAUNCH_FAILED=1
}
RELAUNCH_FAILED=0
CACHE_DIR="$RUN_DIR/relaunch"
# Outside RUN_DIR, which goes with the lock: the shots outlive the run.
SHOT_DIR="$(mktemp -d /tmp/nd-cef-relaunch-shots.XXXXXX)"
echo "relaunch shots: $SHOT_DIR"
export ND_AUTOMATION_CAPTURE=region
launch
relaunch_drive navigate
kill -9 "$HOST_PID" 2>/dev/null || true
wait "$HOST_PID" 2>/dev/null || true
HOST_PID=""
port_free || { echo "FAIL: the killed host's debugging port never came back"; exit 1; }
launch
relaunch_drive relaunch

SECOND_LOG="$RUN_DIR/second.log"
NATIVE_AUTOMATION=1 ND_WEBVIEW_ENGINE=chromium ND_CEF_STYLE=chrome ND_WEBVIEW_TRACE=1 \
  ND_CEF_CACHE="$CACHE_DIR" ND_SCRIPT=examples/webview-probe/cef-chrome.tsx "$HOST" \
  "--load-extension=$EXTENSION" "--remote-debugging-port=$((PORT + 1))" --remote-allow-origins='*' \
  >"$SECOND_LOG" 2>&1 &
SECOND_PID=$!
for _ in $(seq 1 300); do
  grep -qE "ND_CEF_PROFILE_IN_USE|cef_initialize failed" "$SECOND_LOG" && break
  kill -0 "$SECOND_PID" 2>/dev/null || break
  sleep 0.1
done
relaunch_drive collision
if grep -q "ND_CEF_PROFILE_IN_USE" "$SECOND_LOG"; then
  echo "ND_CEF_RELAUNCH_CHECK collision/second: ok ($(grep -m1 ND_CEF_PROFILE_IN_USE "$SECOND_LOG"))"
else
  echo "ND_CEF_RELAUNCH_CHECK collision/second: FAIL (the second host never said the profile is in use:" \
    "$(grep -m1 -E "ND_WARN|ND_WEBVIEW_ENGINE" "$SECOND_LOG" || echo "no engine line")))"
  RELAUNCH_FAILED=1
fi
kill -9 "$SECOND_PID" 2>/dev/null || true
wait "$SECOND_PID" 2>/dev/null || true
SECOND_PID=""

kill "$HOST_PID"
set +e
wait "$HOST_PID"
STATUS=$?
set -e
HOST_PID=""
[ "$STATUS" -eq 0 ] || { echo "FAIL: the relaunched host exited $STATUS instead of cleanly"; exit 1; }
[ "$RELAUNCH_FAILED" -eq 0 ] || { echo "FAIL: relaunch leg"; exit 1; }
echo "ND_CEF_CHROME_RELAUNCH_OK a killed host relaunches and a second host on its cache open no Chromium window"
echo "cef chrome style: OK"
