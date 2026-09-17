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
# Recording grant; without one the capture legs fail rather than being skipped.
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
PROFILE="$(mktemp -d /tmp/nd-app-chrome.XXXXXX)"
mkdir -p "$PROFILE/store" "$PROFILE/cef" "$PROFILE/shots"

# Every new NDShell*.ips under ~/Library/Logs/DiagnosticReports is a crash the
# machine owner sees as a "quit unexpectedly" dialog. The run fails on one even
# if every leg passed.
REPORTS="$HOME/Library/Logs/DiagnosticReports"
crash_reports() {
  ls -1 "$REPORTS" 2>/dev/null | grep -E "^NDShell( Helper.*)?-" | sort
}
BASELINE_REPORTS="$(crash_reports)"
new_crash_reports() {
  comm -13 <(printf '%s\n' "$BASELINE_REPORTS") <(crash_reports) | grep -v '^$' || true
}

HOST_PID=""
cleanup() {
  [ -n "${HOST_PID:-}" ] && kill -9 "$HOST_PID" 2>/dev/null
  # Chromium's helpers outlive a -9'd host; they are children of this run and
  # nobody else's, so they go with it.
  pkill -9 -f "$ROOT/swift/.build/NDShellDev.app" 2>/dev/null
  true
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
    "$HOST" "--load-extension=$EXTENSION" "--remote-debugging-port=$PORT" >"$LOG" 2>&1 &
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

# Quitting with the inspector docked still segfaults inside Chromium's own
# window teardown (docs/webview.md, "Chrome style"). The ordered close is in
# place and the plain quits are clean; these two are killed outright instead,
# so a run leaves no "quit unexpectedly" dialog behind, and reported as the
# open legs they are.
echo "ND_APP_CHROME_QUIT sigterm-devtools: FAILING (killed; the docked quit segfaults)"
echo "ND_APP_CHROME_QUIT cmdq-devtools: FAILING (killed; the docked quit segfaults)"
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
