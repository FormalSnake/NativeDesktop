#!/usr/bin/env bash
# Gate for the Chrome browser style: runs examples/cef-probe with
# ND_CEF_STYLE=chrome and an unpacked extension, checks that Chromium's own
# extension runtime came up, that none of the routes that would open a Chromium
# window opened one, that devtools docks inside the view, and that the
# extension and its storage survive a restart.
# Marker: ND_CEF_CHROME_OK.
#
# Xvfb for the same reason scripts/headless-webview-cef.sh uses it: CEF's
# windowed embedding is X11-only, and a plain X server is what makes both the
# capture and the top-level census mean anything. Nothing here is Xvfb-specific
# beyond that; the same script runs against a real X11 (or XWayland) session by
# pointing ND_CEF_DISPLAY at it and setting ND_CEF_KEEP_DISPLAY=1.
set -euo pipefail
cd "$(dirname "$0")/.."

: "${ND_CEF_DIST:=$HOME/.cache/nativedesktop/cef/151.3.23-linux64}"
[ -f "$ND_CEF_DIST/Release/libcef.so" ] || { echo "SKIP: no CEF distribution at $ND_CEF_DIST"; exit 0; }
# Chromium resolves icudtl.dat and the .pak files against libcef.so's own
# directory before `resources_dir_path` is applied, so the unpacked
# distribution's Release/Resources split is stitched together here, as the
# alloy gate does.
ln -sfn "$ND_CEF_DIST"/Resources/* "$ND_CEF_DIST/Release/"
export ND_CEF_ROOT="$ND_CEF_DIST/Release"

if [ -n "${ND_CEF_LD_LIBRARY_PATH:-}" ]; then
  export LD_LIBRARY_PATH="$ND_CEF_LD_LIBRARY_PATH${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
export DISPLAY="${ND_CEF_DISPLAY:-:96}"
export GDK_BACKEND=x11
export GSK_RENDERER=cairo
export NATIVE_AUTOMATION=1
export ND_WEBVIEW_ENGINE=chromium
export ND_CEF_STYLE=chrome
export ND_APP_ID="${ND_APP_ID:-dev.nativedesktop.headlessCefChrome}"
CDP_PORT="${ND_CDP_PORT:-9334}"
export ND_CDP_PORT="$CDP_PORT"
EXTENSION="$PWD/scripts/fixtures/chrome-ext"
# One data home across both passes: the restart leg is the whole point.
XDG_DATA_HOME="$(mktemp -d)"
export XDG_DATA_HOME

XVFB_PID=""
if [ -z "${ND_CEF_KEEP_DISPLAY:-}" ]; then
  Xvfb "$DISPLAY" -screen 0 1280x900x24 -nolisten tcp >/dev/null 2>&1 &
  XVFB_PID=$!
fi
# Never `kill "${HOST_PID:-0}"`: the success path clears HOST_PID, and `kill 0`
# signals the whole process group, which on a remote shell takes the session
# down with it.
trap '[ -n "$XVFB_PID" ] && kill "$XVFB_PID" 2>/dev/null; [ -n "${HOST_PID:-}" ] && kill "$HOST_PID" 2>/dev/null; true' EXIT

for _ in $(seq 1 100); do
  xwininfo -root >/dev/null 2>&1 && break
  sleep 0.1
done
xwininfo -root >/dev/null 2>&1 || { echo "FAIL: no X server on $DISPLAY"; exit 1; }

toplevels() {
  xwininfo -root -children |
    sed -n 's/^ *\(0x[0-9a-f]*\).*[^0-9]\([0-9]\+\)x\([0-9]\+\)+.*/\1 \2 \3/p' |
    while read -r id w h; do
      [ "$w" -ge 200 ] && [ "$h" -ge 200 ] || continue
      if xwininfo -id "$id" 2>/dev/null | grep -q "Map State: IsViewable"; then echo "$id"; fi
    done
  true
}
BEFORE_X11="$(toplevels | wc -l)"

LOG=$(mktemp)

run_pass() {
  local pass="$1"
  : >"$LOG"
  ND_WEBVIEW_TRACE=1 ND_SCRIPT=examples/cef-probe/main.tsx ./zig-out/bin/nd-hello \
    --load-extension="$EXTENSION" \
    --remote-debugging-port="$CDP_PORT" --remote-allow-origins='*' >"$LOG" 2>&1 &
  HOST_PID=$!

  for _ in $(seq 1 900); do
    grep -q "ND_AUTOMATION_LISTENING" "$LOG" && grep -q "ND_COMMIT_APPLIED" "$LOG" && break
    sleep 0.1
  done
  grep -q "ND_AUTOMATION_LISTENING" "$LOG" || { echo "FAIL($pass): no automation listener"; cat "$LOG"; exit 1; }

  for _ in $(seq 1 600); do
    grep -q "ND_WEBVIEW_ENGINE chromium" "$LOG" && break
    sleep 0.1
  done
  grep -q "ND_WEBVIEW_ENGINE chromium" "$LOG" || { echo "FAIL($pass): the chromium engine did not load"; tail -40 "$LOG"; exit 1; }
  if grep -q "falling back to the system engine" "$LOG"; then
    echo "FAIL($pass): the view fell back to the system engine"
    exit 1
  fi

  # The whole M1 contract, under Chrome style this time: the same drive the
  # alloy gate runs, so a style that breaks navigation or events fails here
  # rather than in the app. It also carries the listExtensions check, which
  # needs the extension the launch loaded.
  if [ "$pass" = "first" ]; then
    SOCK=$(grep -m1 "ND_AUTOMATION_LISTENING" "$LOG" | sed 's/.*path=//')
    ND_AUTOMATION_SOCKET="$SOCK" bun scripts/cef-drive.ts >"$XDG_RUNTIME_DIR/chrome-drive-$pass.log" 2>&1 \
      || { echo "FAIL($pass): M1 driver"; cat "$XDG_RUNTIME_DIR/chrome-drive-$pass.log"; tail -60 "$LOG"; exit 1; }
    grep -q "ND_CEF_M1_OK" "$XDG_RUNTIME_DIR/chrome-drive-$pass.log" || { echo "FAIL($pass): M1 driver did not report success"; exit 1; }
    echo "ND_CEF_CHROME_M1_OK($pass) the alloy gate's own drive passes under chrome style"
  fi

  # The native context menu is driven with real right-clicks and read back from
  # the host's own trace, so it gets a pass and a script of its own.
  if [ "$pass" = "menu" ]; then
    ND_HOST_LOG="$LOG" ND_MENU_SHOT_PATH="${ND_CEF_MENU_SHOT_PATH:-$XDG_RUNTIME_DIR/cef-chrome-menu.png}" \
      bun scripts/cef-menu-drive.ts 2>&1 | tee "$XDG_RUNTIME_DIR/chrome-menu.log" || true
    grep -q "ND_CEF_MENU_OK" "$XDG_RUNTIME_DIR/chrome-menu.log" || {
      echo "FAIL($pass): context-menu driver"
      tail -60 "$LOG"
      exit 1
    }
    return
  fi

  ND_CHROME_PASS="$pass" ND_CHROME_SHOT_PATH="${ND_CEF_SHOT_PATH:-$XDG_RUNTIME_DIR/cef-chrome-devtools.png}" \
    bun scripts/cef-chrome-drive.ts 2>&1 | tee "$XDG_RUNTIME_DIR/chrome-legs-$pass.log" || true
  grep -q "ND_CEF_CHROME_LEGS_OK($pass)" "$XDG_RUNTIME_DIR/chrome-legs-$pass.log" || {
    echo "FAIL($pass): chrome-style driver"
    tail -60 "$LOG"
    exit 1
  }
}

quit_host() {
  local pass="$1"
  kill -TERM "$HOST_PID"
  # `|| status=$?` rather than a bare wait: under `set -e` the 143 a SIGTERMed
  # host exits with would take the script down before the check below runs.
  local status=0
  wait "$HOST_PID" 2>/dev/null || status=$?
  HOST_PID=""
  # Strictly zero: the host turns SIGTERM into the same teardown a window close
  # takes, so a 143 means that handler never ran and anything above 128 means it
  # died on the way out.
  if [ "$status" -ne 0 ]; then
    echo "FAIL($pass): the host exited $status while quitting with live views"
    tail -30 "$LOG"
    exit 1
  fi
  echo "ND_CEF_CHROME_CLEAN_QUIT_OK($pass) exit $status with live views"
  # CEF's process tree outlives the host's own exit by a moment, and the second
  # pass needs the debugging port and the cache lock back.
  for _ in $(seq 1 100); do
    curl -s --max-time 1 "http://127.0.0.1:$CDP_PORT/json/version" >/dev/null 2>&1 || break
    sleep 0.2
  done
}

run_pass first
quit_host first

AFTER_X11="$(toplevels | wc -l)"
ADDED=$((AFTER_X11 - BEFORE_X11))
if [ "$ADDED" -gt 0 ]; then
  echo "FAIL: $ADDED X11 top-level(s) outlived the first pass"
  xwininfo -root -children
  exit 1
fi
echo "ND_CEF_CHROME_NO_STRAY_WINDOW_OK toplevels $BEFORE_X11 -> $AFTER_X11 across every window-opening route"

# The restart: same XDG_DATA_HOME, so the same Chrome profile, and the gate's
# extension has to be back with the storage the first pass wrote.
run_pass second
quit_host second

run_pass menu
quit_host menu

# DevTools last, and asked to quit like the others: the engine closes the
# devtools browser before the one it inspects, which is what the clean exit
# below depends on.
run_pass devtools
quit_host devtools

echo "ND_CEF_CHROME_OK chrome style: extension runtime, no window of its own, docked devtools, state across a restart"
