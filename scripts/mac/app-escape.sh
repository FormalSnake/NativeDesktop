#!/usr/bin/env bash
# No-escape gate against the REAL browser app, on macOS: every key chord, page
# API, context menu item, chrome:// control and extension call that can ask
# Chromium for a window, a tab strip, a bubble or its app menu, driven through
# scripts/mac/app-escape-drive.ts. Marker: ND_APP_ESCAPE_MAC_OK.
#
# Holds the machine-wide mac CEF lock for the whole run (the drive moves the
# owner's real cursor), and gives CEF a fresh cache and its own debugging port.
# A route that leaves a Chromium window on screen ends that host; the gate
# relaunches and carries on from the next route, so one escape does not poison
# every route after it.
#
# ND_APP_DIR points at the app checkout (default ~/Developer/nativebrowser).
# ND_ESCAPE_GROUPS picks route groups (key, page, menu, webui, ext),
# ND_ESCAPE_ROUTES single routes, ND_ESCAPE_EXPLORE=1 reports without failing.
# Other gates wait on the same lock, so one run holds it for at most
# ND_ESCAPE_HOLD_S seconds (default 540); run the groups one at a time.
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT="$(pwd)"

APP_DIR="${ND_APP_DIR:-$HOME/Developer/nativebrowser}"
[ -f "$APP_DIR/src/main.tsx" ] || { echo "FAIL: no app at $APP_DIR (set ND_APP_DIR)"; exit 1; }

# Built before the lock is taken: the build puts nothing on screen.
HOST="$(env -u SDKROOT -u DEVELOPER_DIR ./scripts/mac/dev-cef-bundle.sh | tail -1)"
INPUT="$ROOT/swift/.build/release/NDShell"
"$INPUT" --nd-grant >/dev/null 2>&1 || echo "ND_WARN --nd-grant failed for $INPUT; app.cursor needs Accessibility"

LOCK_LIB="${ND_MAC_GATE_LOCK_LIB:-$ROOT/scripts/mac/cef-gate-lock.sh}"
# shellcheck source=/dev/null
. "$LOCK_LIB"
cef_gate_lock
HOLD_UNTIL=$(( $(date +%s) + ${ND_ESCAPE_HOLD_S:-540} ))
HOST_PID=""
cleanup() {
  if [ -n "${HOST_PID:-}" ]; then kill -9 "$HOST_PID" 2>/dev/null || true; fi
  pkill -9 -f "$ROOT/swift/.build/NDShellDev.app" 2>/dev/null || true
  cef_gate_unlock
}
trap cleanup EXIT

PORT="${ND_CEF_DEBUG_PORT:-9441}"
FIXTURE_PORT="${ND_APP_FIXTURE_PORT:-9731}"
EXTENSIONS="$ROOT/examples/webview-probe/chrome-style-ext,$ROOT/examples/webview-probe/escape-ext"

settle() {
  pkill -9 -f "$ROOT/swift/.build/NDShellDev.app" 2>/dev/null || true
  for _ in $(seq 1 100); do
    pgrep -f "$ROOT/swift/.build/NDShellDev.app" >/dev/null || break
    sleep 0.1
  done
}

LOG=""
RUN=0
launch() {
  settle
  RUN=$((RUN + 1))
  # A fresh profile per host: an escape that got as far as a Chromium window
  # leaves session state behind that the next host would restore.
  local profile="$RUN_DIR/run-$RUN"
  mkdir -p "$profile/store" "$profile/cef"
  LOG="$profile/host.log"
  cd "$APP_DIR"
  NATIVE_AUTOMATION=1 ND_WEBVIEW_ENGINE=chromium ND_CEF_STYLE=chrome ND_WEBVIEW_TRACE=1 \
    ND_SCRIPT=src/main.tsx NB_STORE_DIR="$profile/store" ND_CEF_CACHE="$profile/cef" \
    "$HOST" "--load-extension=$EXTENSIONS" "--remote-debugging-port=$PORT" --remote-allow-origins='*' >"$LOG" 2>&1 &
  HOST_PID=$!
  cd "$ROOT"
  for _ in $(seq 1 600); do
    grep -q "ND_AUTOMATION_LISTENING" "$LOG" 2>/dev/null && break
    sleep 0.1
  done
  grep -q "ND_AUTOMATION_LISTENING" "$LOG" || { echo "FAIL: no automation listener"; tail -40 "$LOG"; exit 1; }
  for _ in $(seq 1 600); do
    grep -q "ND_COMMIT_APPLIED" "$LOG" 2>/dev/null && grep -q "ND_WEBVIEW_ENGINE" "$LOG" && break
    sleep 0.1
  done
  grep -q "ND_WEBVIEW_ENGINE chromium" "$LOG" || { echo "FAIL: the chromium engine did not load"; exit 1; }
  SOCK="$(grep -m1 "ND_AUTOMATION_LISTENING" "$LOG" | sed 's/.*path=//')"
}

# Captures and host logs outlive the run directory, which the lock removes.
SHOTS="${ND_ESCAPE_SHOTS:-/tmp/nd-escape-shots}"
mkdir -p "$SHOTS"
DONE="${ND_ESCAPE_SKIP:-}"
FAILED=0
OUT="$RUN_DIR/drive.out"
for _ in $(seq 1 40); do
  LEFT=$(( HOLD_UNTIL - $(date +%s) - 30 ))
  if [ "$LEFT" -le 30 ]; then echo "ND_ESCAPE_HOLD_SPENT"; FAILED=1; break; fi
  launch
  set +e
  ND_AUTOMATION_SOCKET="$SOCK" ND_HOST_PID="$HOST_PID" ND_CEF_DEBUG_PORT="$PORT" \
    ND_APP_FIXTURE_PORT="$FIXTURE_PORT" ND_APP_HOST_LOG="$LOG" ND_INPUT_BINARY="$INPUT" \
    ND_ESCAPE_SKIP="$DONE" ND_ESCAPE_SHOTS="$SHOTS" ND_ESCAPE_BUDGET_S="$LEFT" ND_NDSHOT="${ND_NDSHOT:-$ROOT/tools/ndshot/bin/ndshot}" \
    timeout --kill-after=10 "$LEFT" bun scripts/mac/app-escape-drive.ts 2>&1 | tee "$OUT"
  STATUS=${PIPESTATUS[0]}
  set -e
  # Every route this host reported is done, whatever it reported.
  for name in $(sed -n 's/^ND_ESCAPE_ROUTE \([^:]*\):.*/\1/p' "$OUT"); do DONE="$DONE,$name"; done
  if grep -q "^ND_ESCAPE_ROUTE .*: \(ESCAPE\|SETUP\|HOST DIED\)" "$OUT"; then FAILED=1; fi
  if grep -q "^ND_ESCAPE_UNRUN" "$OUT"; then FAILED=1; fi
  kill -9 "$HOST_PID" 2>/dev/null || true
  wait "$HOST_PID" 2>/dev/null || true
  HOST_PID=""
  cp "$LOG" "$SHOTS/host-$RUN.log" 2>/dev/null || true
  # 3 is the drive stopping on a Chromium window; anything else is the end.
  [ "$STATUS" -eq 3 ] || break
done
[ "${STATUS:-1}" -eq 0 ] || [ "${STATUS:-1}" -eq 3 ] || FAILED=1

if [ "${ND_ESCAPE_EXPLORE:-}" = "1" ]; then echo "ND_APP_ESCAPE_EXPLORED"; exit 0; fi
[ "$FAILED" -eq 0 ] || { echo "app no-escape: FAILED"; exit 1; }
echo "ND_APP_ESCAPE_MAC_OK"
