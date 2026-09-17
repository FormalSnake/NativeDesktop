#!/usr/bin/env bash
# Acceptance gate for a real app under `webview.cef.style: "chrome"`, in the two
# shapes of session a Linux user actually runs: an X11 desktop with a
# reparenting window manager, and a wlroots compositor with XWayland. Both are
# headless here. Marker: ND_APP_CHROME_OK.
#
# The bare-Xvfb gates (scripts/headless-webview-cef*.sh) run with no window
# manager and never resize anything, which is how a browser whose X window
# stayed at its creation size passed them. Everything below is driven with real
# X/compositor input and asserted against three sources at once: the X window
# tree, CDP against the page, and captures.
#
# ND_APP_DIR points at the app checkout to run (default: this repo's cef-probe
# example, which carries none of the app-level legs and reports them as skips).
set -euo pipefail
cd "$(dirname "$0")/.."
FRAMEWORK="$PWD"

: "${ND_CEF_DIST:=$HOME/.cache/nativedesktop/cef/151.3.23-linux64}"
[ -f "$ND_CEF_DIST/Release/libcef.so" ] || { echo "SKIP: no CEF distribution at $ND_CEF_DIST"; exit 0; }
ln -sfn "$ND_CEF_DIST"/Resources/* "$ND_CEF_DIST/Release/"
export ND_CEF_ROOT="$ND_CEF_DIST/Release"
[ -x "$FRAMEWORK/zig-out/bin/nd-hello" ] || { echo "FAIL: build the host first (zig build)"; exit 1; }

if [ -n "${ND_CEF_LD_LIBRARY_PATH:-}" ]; then
  export LD_LIBRARY_PATH="$ND_CEF_LD_LIBRARY_PATH${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi

APP_DIR="${ND_APP_DIR:-$FRAMEWORK}"
APP_SCRIPT="${ND_APP_SCRIPT:-}"
if [ -z "$APP_SCRIPT" ]; then
  if [ "$APP_DIR" = "$FRAMEWORK" ]; then APP_SCRIPT="examples/cef-probe/main.tsx"; else APP_SCRIPT="src/main.tsx"; fi
fi
[ -f "$APP_DIR/$APP_SCRIPT" ] || { echo "FAIL: no $APP_SCRIPT under $APP_DIR"; exit 1; }

RIGS="${ND_ACCEPT_RIGS:-x11 wlr}"
SCALE="${ND_ACCEPT_SCALE:-1}"
DISPLAY_NUM="${ND_ACCEPT_DISPLAY:-:99}"
CDP_PORT="${ND_CDP_PORT:-9555}"
FIXTURE_PORT="${ND_ACCEPT_FIXTURE_PORT:-9557}"
WORK="$(mktemp -d)"
SHOTS="${ND_ACCEPT_SHOTS:-$WORK/shots}"
mkdir -p "$SHOTS"
echo "work dir $WORK, captures in $SHOTS"

# Every background process this script starts, by pid, so the trap kills what it
# owns and nothing else. Other agents share this machine.
PIDS=()
cleanup() {
  for pid in "${PIDS[@]:-}"; do
    [ -n "$pid" ] || continue
    kill "$pid" 2>/dev/null || true
  done
}
trap cleanup EXIT

bun "$FRAMEWORK/scripts/app-chrome-fixture.ts" "$FIXTURE_PORT" >"$WORK/fixture.log" 2>&1 &
PIDS+=($!)
FIXTURE="http://127.0.0.1:$FIXTURE_PORT/"
for _ in $(seq 1 100); do
  curl -s --max-time 1 "$FIXTURE" >/dev/null 2>&1 && break
  sleep 0.1
done
curl -s --max-time 1 "$FIXTURE" >/dev/null || { echo "FAIL: the fixture server never answered on $FIXTURE"; cat "$WORK/fixture.log"; exit 1; }

# The app under test starts on the fixture with two tabs, so no leg depends on
# typing an address before it can assert anything, and the tab-switch legs have
# a second live view. An app that does not use @nativedesktop/react's store
# ignores this and starts wherever it starts.
seed_store() {
  local dir="$1"
  mkdir -p "$dir"
  cat >"$dir/session.json" <<EOF
{"version":1,"data":{"tabs":[{"id":"t1","url":"$FIXTURE","title":"","pinned":false},{"id":"t2","url":"$FIXTURE?two","title":"","pinned":false}],"activeId":"t1","nextTabId":3,"windowWidth":1280,"windowHeight":800,"zoomByHost":{}}}
EOF
  cat >"$dir/settings.json" <<'EOF'
{"version":1,"data":{"searchEngine":"duckduckgo","homepage":"","restoreOnLaunch":true,"layout":"sidebar"}}
EOF
}

# One rig's worth of environment, exported for both the host and the drive.
launch_host() {
  local rig="$1" log="$2"
  seed_store "$WORK/$rig/store"
  ( cd "$APP_DIR"
    export XDG_RUNTIME_DIR="$WORK/$rig/xdg"
    export XDG_DATA_HOME="$WORK/$rig/data"
    export NB_STORE_DIR="$WORK/$rig/store"
    export GDK_BACKEND=x11
    export GSK_RENDERER=cairo
    export GDK_SCALE="$SCALE"
    export NATIVE_AUTOMATION=1
    export ND_WEBVIEW_ENGINE=chromium
    export ND_CEF_STYLE=chrome
    export ND_SCRIPT="$APP_SCRIPT"
    export ND_WEBVIEW_TRACE=1
    # A distinct id per rig: both rigs run at once against one session bus, and
    # the second launch would otherwise activate the first app and exit.
    export ND_APP_ID="dev.nativedesktop.ndAccept${rig}"
    # setsid so the whole CEF process tree lands in one session, which is what
    # the orphan check at quit counts.
    exec setsid "$FRAMEWORK/zig-out/bin/nd-hello" \
      --remote-debugging-port="$CDP_PORT" --remote-allow-origins='*'
  ) >"$log" 2>&1 &
  HOST_PID=$!
  PIDS+=("$HOST_PID")
}

wait_for_host() {
  local log="$1"
  for _ in $(seq 1 1200); do
    grep -q "ND_AUTOMATION_LISTENING" "$log" && curl -s --max-time 1 "http://127.0.0.1:$CDP_PORT/json/version" >/dev/null 2>&1 && return 0
    sleep 0.1
  done
  echo "FAIL: the host never came up"
  tail -40 "$log"
  return 1
}

run_rig() {
  local rig="$1"
  local log="$WORK/$rig/host.log"
  mkdir -p "$WORK/$rig/xdg" "$WORK/$rig/data" "$SHOTS/$rig"
  chmod 700 "$WORK/$rig/xdg"

  case "$rig" in
    x11)
      Xvfb "$DISPLAY_NUM" -screen 0 1920x1200x24 -nolisten tcp >"$WORK/x11/xvfb.log" 2>&1 &
      PIDS+=($!)
      export DISPLAY="$DISPLAY_NUM"
      for _ in $(seq 1 200); do xwininfo -root >/dev/null 2>&1 && break; sleep 0.1; done
      xwininfo -root >/dev/null 2>&1 || { echo "FAIL: no X server on $DISPLAY_NUM"; return 1; }
      # A reparenting, EWMH window manager. Without one nothing ever resizes,
      # maximizes or fullscreens a toplevel, which is the whole point here.
      openbox >"$WORK/x11/openbox.log" 2>&1 &
      PIDS+=($!)
      sleep 1
      unset SWAYSOCK WAYLAND_DISPLAY
      ;;
    wlr)
      local swayrt="$WORK/wlr/swayrt"
      mkdir -p "$swayrt"; chmod 700 "$swayrt"
      cat >"$WORK/wlr/sway.conf" <<EOF
xwayland force
output HEADLESS-1 mode 1920x1200
default_border none
default_floating_border none
focus_follows_mouse no
exec sh -c 'printf "%s" "\$DISPLAY" > $WORK/wlr/xdisplay'
EOF
      ( export XDG_RUNTIME_DIR="$swayrt"
        export WLR_BACKENDS=headless
        export WLR_LIBINPUT_NO_DEVICES=1
        export WLR_RENDERER=pixman
        unset DISPLAY WAYLAND_DISPLAY
        exec sway -c "$WORK/wlr/sway.conf"
      ) >"$WORK/wlr/sway.log" 2>&1 &
      PIDS+=($!)
      for _ in $(seq 1 200); do [ -s "$WORK/wlr/xdisplay" ] && break; sleep 0.1; done
      [ -s "$WORK/wlr/xdisplay" ] || { echo "FAIL: sway never started XWayland"; tail -20 "$WORK/wlr/sway.log"; return 1; }
      export DISPLAY="$(cat "$WORK/wlr/xdisplay")"
      export XDG_RUNTIME_DIR="$swayrt"
      export WAYLAND_DISPLAY=wayland-1
      export SWAYSOCK="$(ls "$swayrt"/sway-ipc.*.sock | head -1)"
      ;;
    *) echo "FAIL: unknown rig $rig"; return 1 ;;
  esac

  launch_host "$rig" "$log"
  wait_for_host "$log" || return 1
  local sock
  sock="$(grep -m1 "ND_AUTOMATION_LISTENING" "$log" | sed 's/.*path=//')"

  ND_ACCEPT_RIG="$rig" ND_AUTOMATION_SOCKET="$sock" ND_CDP_PORT="$CDP_PORT" \
    ND_ACCEPT_FIXTURE="$FIXTURE" ND_ACCEPT_SHOTS="$SHOTS/$rig" ND_ACCEPT_SCALE="$SCALE" \
    ND_ACCEPT_HOST_LOG="$log" \
    bun "$FRAMEWORK/scripts/app-chrome-drive.ts" 2>&1 | tee "$WORK/$rig/drive.log" || true

  # Quit last, and asserted, except on a run that could not shut the inspector
  # again: a Chrome-style browser that has had devtools open does not come back
  # from SIGTERM at all (docs/webview.md), which is a leg of its own rather
  # than something the quit assertion should absorb.
  if grep -q "ND_APP_CHROME_DEVTOOLS_OPEN" "$WORK/$rig/drive.log"; then
    echo "  quit($rig): skip (the inspector was still open; see the devToolsCloses leg)"
    kill -KILL "$HOST_PID" 2>/dev/null || true
    wait "$HOST_PID" 2>/dev/null || true
    grep -q "ND_APP_CHROME_LEGS_OK($rig)" "$WORK/$rig/drive.log" || { echo "ND_APP_CHROME_FAIL($rig)"; return 1; }
    return 0
  fi
  kill -TERM "$HOST_PID"
  # Bounded: a Chrome-style browser that still has devtools open does not come
  # back from SIGTERM at all, and an unbounded wait turns that into a gate that
  # never finishes instead of a leg that fails.
  local waited=0
  while kill -0 "$HOST_PID" 2>/dev/null && [ "$waited" -lt 200 ]; do
    sleep 0.1
    waited=$((waited + 1))
  done
  if kill -0 "$HOST_PID" 2>/dev/null; then
    echo "  quit($rig): FAIL (still alive 20s after SIGTERM)"
    kill -KILL "$HOST_PID" 2>/dev/null || true
    wait "$HOST_PID" 2>/dev/null || true
    echo "ND_APP_CHROME_FAIL($rig)"
    return 1
  fi
  local status=0
  wait "$HOST_PID" 2>/dev/null || status=$?
  if [ "$status" -ne 0 ] && [ "$status" -ne 143 ]; then
    echo "  quit($rig): FAIL (host exited $status)"
    tail -30 "$log"
    echo "ND_APP_CHROME_FAIL($rig)"
    return 1
  fi
  for _ in $(seq 1 150); do
    pgrep -s "$HOST_PID" >/dev/null 2>&1 || break
    sleep 0.2
  done
  local orphans
  orphans="$(pgrep -s "$HOST_PID" 2>/dev/null | wc -l || true)"
  if [ "$orphans" -ne 0 ]; then
    echo "  quit($rig): FAIL ($orphans process(es) left in the host's session)"
    pgrep -s "$HOST_PID" -a 2>/dev/null || true
    echo "ND_APP_CHROME_FAIL($rig)"
    return 1
  fi
  echo "  quit($rig): ok (exit $status, no process left in the session)"

  grep -q "ND_APP_CHROME_LEGS_OK($rig)" "$WORK/$rig/drive.log" || { echo "ND_APP_CHROME_FAIL($rig)"; return 1; }
  return 0
}

FAILED=0
for rig in $RIGS; do
  echo "== rig $rig (scale $SCALE) =="
  run_rig "$rig" || FAILED=1
  cleanup
  PIDS=()
  # The debugging port and the CEF cache lock come back a moment after the host
  # itself is gone, and the next rig reuses both.
  for _ in $(seq 1 150); do
    curl -s --max-time 1 "http://127.0.0.1:$CDP_PORT/json/version" >/dev/null 2>&1 || break
    sleep 0.2
  done
  bun "$FRAMEWORK/scripts/app-chrome-fixture.ts" "$FIXTURE_PORT" >>"$WORK/fixture.log" 2>&1 &
  PIDS+=($!)
  for _ in $(seq 1 100); do curl -s --max-time 1 "$FIXTURE" >/dev/null 2>&1 && break; sleep 0.1; done
done

[ "$FAILED" -eq 0 ] || { echo "ND_APP_CHROME_FAIL one or more rigs failed; captures in $SHOTS"; exit 1; }
echo "ND_APP_CHROME_OK chrome style holds under a reparenting X11 wm and under wlroots/XWayland"
