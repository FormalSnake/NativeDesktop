#!/usr/bin/env bash
# Moving one live `<webview>` between two host windows, on both CEF styles.
# `moveNode` relocates the native widget without React unmounting it, which is
# how a tab dragged to another window keeps the page it is showing; the lift
# Chrome style does makes that a question about the anchor window as well.
# Marker: ND_CEF_REPARENT_MAC_OK.
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT="$(pwd)"

PORT="${ND_CEF_DEBUG_PORT:-9337}"
HOST="$(env -u SDKROOT -u DEVELOPER_DIR ./scripts/mac/dev-cef-bundle.sh | tail -1)"
PROFILE="$(mktemp -d /tmp/nd-cef-reparent.XXXXXX)"

REPORTS="$HOME/Library/Logs/DiagnosticReports"
crash_reports() { ls -1 "$REPORTS" 2>/dev/null | grep -E "^NDShell( Helper.*)?-" | sort; }
BASELINE_REPORTS="$(crash_reports)"

# ScreenCaptureKit through the signed `ndshot` binary is the only capture path
# that works here: `screencapture` runs as the calling terminal and a terminal
# cannot be granted Screen Recording. Build it if the tree has no copy, and say
# what the grant looks like, because a capture leg the machine cannot run has to name
# that rather than report a picture it never took.
ensure_ndshot() {
  NDSHOT="${ND_NDSHOT:-$ROOT/tools/ndshot/bin/ndshot}"
  if [ ! -x "$NDSHOT" ]; then
    echo "building ndshot (no copy at $NDSHOT)"
    ( cd "$ROOT/tools/ndshot" && ./build.sh >/dev/null ) || {
      echo "FAIL: tools/ndshot/build.sh failed; the capture legs cannot run"; return 1
    }
    NDSHOT="$ROOT/tools/ndshot/bin/ndshot"
  fi
  export ND_NDSHOT="$NDSHOT"
  if ! "$NDSHOT" doctor >/dev/null 2>&1; then
    echo "ND_WARN no Screen Recording grant for $NDSHOT; the capture legs report skip until it is granted"
    echo "       (System Settings > Privacy & Security > Screen Recording, then re-run)"
  fi
}

HOST_PID=""
# Every line guarded: the trap runs under `set -e`, so a test that is merely
# false (no host left to kill, no helper left to sweep) aborts the trap there
# and that status becomes the script's own. The gate then reports a failure
# after printing its OK marker.
cleanup() {
  if [ -n "${HOST_PID:-}" ]; then kill -9 "$HOST_PID" 2>/dev/null || true; fi
  pkill -9 -f "$ROOT/swift/.build/NDShellDev.app" 2>/dev/null || true
}
trap cleanup EXIT

settle() {
  pkill -9 -f "$ROOT/swift/.build/NDShellDev.app" 2>/dev/null || true
  for _ in $(seq 1 100); do
    pgrep -f "$ROOT/swift/.build/NDShellDev.app" >/dev/null || break
    sleep 0.1
  done
}

ensure_ndshot || exit 1

FAILED=0
run_style() {
  local style="$1"
  settle
  LOG="$PROFILE/host-$style.log"
  NATIVE_AUTOMATION=1 ND_WEBVIEW_ENGINE=chromium ND_CEF_STYLE="$style" ND_WEBVIEW_TRACE=1 \
    ND_CEF_CACHE="$PROFILE/cef-$style" ND_SCRIPT=examples/webview-probe/cef-reparent.tsx \
    "$HOST" "--remote-debugging-port=$PORT" >"$LOG" 2>&1 &
  HOST_PID=$!
  for _ in $(seq 1 600); do
    grep -q "ND_AUTOMATION_LISTENING" "$LOG" 2>/dev/null && grep -q "ND_COMMIT_APPLIED" "$LOG" && break
    sleep 0.1
  done
  grep -q "ND_AUTOMATION_LISTENING" "$LOG" || { echo "FAIL[$style]: no automation listener"; tail -20 "$LOG"; return 1; }
  grep -q "ND_WEBVIEW_ENGINE chromium" "$LOG" || { echo "FAIL[$style]: the chromium engine did not load"; return 1; }
  local sock
  sock="$(grep -m1 "ND_AUTOMATION_LISTENING" "$LOG" | sed 's/.*path=//')"
  echo "-- $style"
  local drive_status=0
  ND_AUTOMATION_SOCKET="$sock" ND_HOST_PID="$HOST_PID" ND_CEF_DEBUG_PORT="$PORT" \
    ND_CEF_STYLE="$style" ND_CEF_REPARENT_DEVTOOLS="${ND_CEF_REPARENT_DEVTOOLS:-0}" \
    ND_APP_SHOTS="$PROFILE" bun scripts/mac/cef-reparent-drive.ts || drive_status=1
  [ "$drive_status" -eq 0 ] || FAILED=1
  # The inspector leg leaves the dock open and quitting on that still segfaults
  # inside Chromium's window teardown, so that pass is killed rather than
  # quit; the leg is reported as failing either way.
  if [ "${ND_CEF_REPARENT_DEVTOOLS:-0}" = "1" ]; then
    kill -9 "$HOST_PID" 2>/dev/null || true
    wait "$HOST_PID" 2>/dev/null || true
    HOST_PID=""
    settle
    return 0
  fi
  kill -TERM "$HOST_PID"
  for _ in $(seq 1 300); do kill -0 "$HOST_PID" 2>/dev/null || break; sleep 0.1; done
  if kill -0 "$HOST_PID" 2>/dev/null; then
    echo "FAIL[$style]: the host did not exit on SIGTERM"
    kill -9 "$HOST_PID"
    FAILED=1
  else
    set +e
    wait "$HOST_PID" 2>/dev/null
    local status=$?
    set -e
    [ "$status" -eq 0 ] || { echo "FAIL[$style]: the host exited $status"; FAILED=1; }
  fi
  HOST_PID=""
  settle
}

run_style chrome || FAILED=1
run_style alloy || FAILED=1

# The inspector pass, on Chrome style only: Alloy gives DevTools a window of
# its own and has nothing to carry across the move.
echo "-- chrome, inspector across the move"
ND_CEF_REPARENT_DEVTOOLS=1 run_style chrome || FAILED=1

NEW_CRASHES="$(comm -13 <(printf '%s\n' "$BASELINE_REPORTS") <(crash_reports) | grep -v '^$' || true)"
if [ -n "$NEW_CRASHES" ]; then
  echo "FAIL: the run left crash reports behind:"
  printf '  %s\n' $NEW_CRASHES
  FAILED=1
fi

[ "$FAILED" -eq 0 ] || { echo "cef reparent: FAILED"; exit 1; }
echo "ND_CEF_REPARENT_MAC_OK"
