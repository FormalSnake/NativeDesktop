#!/usr/bin/env bash
# Chrome style gate against the REAL browser app, on macOS. Assembles the dev
# CEF bundle, launches the app checkout's own src/main.tsx under
# ND_CEF_STYLE=chrome with a throwaway profile, and drives it with
# scripts/mac/app-chrome-drive.ts. Marker: ND_APP_CHROME_MAC_OK.
#
# The probe gate (scripts/mac/cef-chrome-style.sh) covers one static view in a
# one-window example. This covers what only an app puts on top of Chrome style:
# window resize and fullscreen, several tabs with a lift that has to follow the
# one on show, the app's own accelerators against Chromium's, its popovers over
# the web contents, and the four quit paths.
#
# ND_APP_DIR points at the app checkout (default ~/Developer/nativebrowser).
# ND_NDSHOT points at a copy of tools/ndshot/bin/ndshot that holds the Screen
# Recording grant; without one the capture legs report skip rather than a result.
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT="$(pwd)"

APP_DIR="${ND_APP_DIR:-$HOME/Developer/nativebrowser}"
[ -f "$APP_DIR/src/main.tsx" ] || { echo "FAIL: no app at $APP_DIR (set ND_APP_DIR)"; exit 1; }

PORT="${ND_CEF_DEBUG_PORT:-9436}"
FIXTURE_PORT="${ND_APP_FIXTURE_PORT:-9723}"
EXTENSION="$ROOT/examples/webview-probe/chrome-style-ext"
HOST="$(env -u SDKROOT -u DEVELOPER_DIR ./scripts/mac/dev-cef-bundle.sh | tail -1)"

# One throwaway tree for the whole run: the app's session/settings/history
# store and CEF's profile both hang off it, so nothing here can reach the
# machine owner's own browser data.
# shellcheck source=scripts/mac/cef-gate-lock.sh
. scripts/mac/cef-gate-lock.sh
trap cef_gate_unlock EXIT
cef_gate_lock
PROFILE="$RUN_DIR"
mkdir -p "$PROFILE/store" "$PROFILE/cef" "$PROFILE/shots"

# Every new NDShell*.ips is a crash the machine owner sees as a "quit
# unexpectedly" dialog. The run fails on one even if every leg passed.
# ReportCrash writes to the system directory on this macOS rather than the
# user one, so both are read.
REPORTS="$HOME/Library/Logs/DiagnosticReports /Library/Logs/DiagnosticReports"
crash_reports() {
  for dir in $REPORTS; do
    ls -1 "$dir" 2>/dev/null | grep -E "^NDShell( Helper.*)?-" | sed "s|^|$dir/|" || true
  done | sort
}
BASELINE_REPORTS="$(crash_reports)"
new_crash_reports() {
  comm -13 <(printf '%s\n' "$BASELINE_REPORTS") <(crash_reports) | grep -v '^$' || true
}

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
# false (no host left to kill) aborts the trap there and that status becomes
# the script's own. The gate then reports a failure after printing its OK
# marker.
cleanup() {
  if [ -n "${HOST_PID:-}" ]; then kill -9 "$HOST_PID" 2>/dev/null || true; fi
  # Chromium's helpers outlive a -9'd host; they are children of this run and
  # nobody else's, so they go with it.
  pkill -9 -f "$ROOT/swift/.build/NDShellDev.app" 2>/dev/null || true
  cef_gate_unlock
}
trap cleanup EXIT

# A relaunch before the previous host's helpers have gone hits a CEF cache the
# old process still holds, and cef_initialize refuses.
settle() {
  pkill -9 -f "$ROOT/swift/.build/NDShellDev.app" 2>/dev/null || true
  for _ in $(seq 1 100); do
    pgrep -f "$ROOT/swift/.build/NDShellDev.app" >/dev/null || break
    sleep 0.1
  done
}

LOG=""
launch() {
  settle
  LOG="$PROFILE/host-$1.log"
  # Launched from this shell, not a subshell: $! has to be the host itself or
  # `wait` cannot read its exit status and every pid-keyed check (the window
  # census, the helper sweep) misses it.
  cd "$APP_DIR"
  NATIVE_AUTOMATION=1 ND_WEBVIEW_ENGINE=chromium ND_CEF_STYLE=chrome ND_WEBVIEW_TRACE=1 \
    ND_SCRIPT=src/main.tsx NB_STORE_DIR="$PROFILE/store" ND_CEF_CACHE="$PROFILE/cef" \
    "$HOST" "--load-extension=$EXTENSION" "--remote-debugging-port=$PORT" --remote-allow-origins='*' >"$LOG" 2>&1 &
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
  grep -q "ND_WEBVIEW_ENGINE chromium" "$LOG" || {
    echo "FAIL: the chromium engine did not load"; grep -E "ND_WARN|ND_WEBVIEW_ENGINE" "$LOG" | tail -10; exit 1
  }
  SOCK="$(grep -m1 "ND_AUTOMATION_LISTENING" "$LOG" | sed 's/.*path=//')"
}

drive() {
  ND_AUTOMATION_SOCKET="$SOCK" ND_HOST_PID="$HOST_PID" ND_CEF_DEBUG_PORT="$PORT" \
    ND_APP_FIXTURE_PORT="$FIXTURE_PORT" ND_APP_SHOTS="$PROFILE/shots" ND_APP_HOST_LOG="$LOG" \
    "$@" bun scripts/mac/app-chrome-drive.ts
}

# Asserts the host is gone, took exit 0 with it, and left no Chromium helper
# of this run behind. A helper that outlives the host is a browser that never
# reached cef_shutdown.
expect_clean_exit() {
  local what="$1"
  for _ in $(seq 1 300); do kill -0 "$HOST_PID" 2>/dev/null || break; sleep 0.1; done
  if kill -0 "$HOST_PID" 2>/dev/null; then
    echo "FAIL: $what left the host running"
    tail -20 "$LOG" | grep -vE "^\[[0-9]+:" || true
    kill -9 "$HOST_PID" 2>/dev/null || true
    return 1
  fi
  set +e
  wait "$HOST_PID" 2>/dev/null
  local status=$?
  set -e
  HOST_PID=""
  if [ "$status" -ne 0 ]; then
    echo "FAIL: $what exited $status"
    tail -20 "$LOG" | grep -vE "^\[[0-9]+:" || true
    return 1
  fi
  local orphans
  # Chromium's helpers unwind after the host's own exit; a couple of seconds is
  # the difference between a slow teardown and one that never happened.
  for _ in $(seq 1 200); do
    pgrep -f "$ROOT/swift/.build/NDShellDev.app" >/dev/null || break
    sleep 0.1
  done
  orphans="$(pgrep -f "$ROOT/swift/.build/NDShellDev.app" | tr '\n' ' ')"
  if [ -n "$orphans" ]; then
    echo "FAIL: $what left helper process(es) behind: $orphans"
    pkill -9 -f "$ROOT/swift/.build/NDShellDev.app" 2>/dev/null || true
    return 1
  fi
  echo "ND_APP_CHROME_QUIT $what: ok"
}

ensure_ndshot || exit 1

FAILED=0

launch legs
drive || FAILED=1
kill -9 "$HOST_PID" 2>/dev/null || true
wait "$HOST_PID" 2>/dev/null || true
HOST_PID=""
settle

# The four quit paths, each on a host of its own: a SIGTERM and a cmd+Q, with
# and without the inspector docked.
quit_leg() {
  local name="$1" prep="$2" how="$3"
  launch "$name"
  ND_APP_CHROME_PREP="$prep" drive >/dev/null || { echo "FAIL: $name could not reach its quit state"; FAILED=1; return; }
  if [ "$how" = "sigterm" ]; then
    kill -TERM "$HOST_PID"
  else
    # cmd+Q goes to the frontmost app, so the host is brought forward first.
    osascript -e "tell application \"System Events\" to set frontmost of (first process whose unix id is $HOST_PID) to true" \
      -e "delay 0.4" \
      -e "tell application \"System Events\" to keystroke \"q\" using command down"
  fi
  expect_clean_exit "$name" || FAILED=1
  settle
}

quit_leg "sigterm-plain" plain sigterm
quit_leg "cmdq-plain" plain cmdq

# Quitting after the inspector has been docked still faults inside Chromium's
# own activation path (`-[NSWindow becomeKeyWindow]` reaching a Chromium
# observer), with the ordered close in place and the inspector long closed. The
# pass is killed rather than quit, so a run leaves the machine owner no "quit
# unexpectedly" dialog, and both legs stay reported as failing.
echo "ND_APP_CHROME_QUIT sigterm-devtools: FAILING (killed; a quit after the inspector segfaults)"
echo "ND_APP_CHROME_QUIT cmdq-devtools: FAILING (killed; a quit after the inspector segfaults)"
launch "devtools-open"
ND_APP_CHROME_PREP=devtools drive >/dev/null || echo "FAIL: the docked quit leg could not reach its state"
kill -9 "$HOST_PID" 2>/dev/null || true
wait "$HOST_PID" 2>/dev/null || true
HOST_PID=""
settle
FAILED=1

NEW_CRASHES="$(new_crash_reports)"
if [ -n "$NEW_CRASHES" ]; then
  echo "FAIL: the run left crash reports behind:"
  printf '  %s\n' $NEW_CRASHES
  FAILED=1
fi

[ "$FAILED" -eq 0 ] || { echo "app chrome style: FAILED"; exit 1; }
echo "ND_APP_CHROME_MAC_OK"
