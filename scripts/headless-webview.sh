#!/usr/bin/env bash
# Headless gate for the webview browser/extension surface: runs
# examples/webview-probe under weston and drives it with
# scripts/webview-drive.ts. Marker: ND_WEBVIEW2_OK.
set -euo pipefail
cd "$(dirname "$0")/.."

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
export WAYLAND_DISPLAY=nd-headless-webview
export GSK_RENDERER=cairo
export GDK_BACKEND=wayland
export NATIVE_AUTOMATION=1
# Own application id: GApplication is single-instance per id on the session
# bus, so a gate sharing `dev.nativedesktop.hello` with anything else running
# on the machine exits with ND_ALREADY_RUNNING instead of starting.
export ND_APP_ID="${ND_APP_ID:-dev.nativedesktop.headlessWebview}"
# The cookie-persistence check asserts a file under the user data dir, and the
# jar it writes must not be the dev box's own. Own data dir, cleared per run.
XDG_DATA_HOME="$(mktemp -d)"
export XDG_DATA_HOME
# Scripted answers for the page dialogs the probe raises, consumed in call
# order (alert, confirm, prompt). Without this the host asks a real question
# nobody is there to answer, which is correct behaviour and a hung gate.
export ND_AUTOMATION_DIALOG_SCRIPT='{"webview.scriptDialog":[{"accepted":true},{"accepted":true},{"accepted":true,"text":"nd-typed"}]}'

weston --backend=headless --socket="$WAYLAND_DISPLAY" --idle-time=0 &
WESTON_PID=$!
# Never `kill "${HOST_PID:-0}"`: the success path clears HOST_PID, and `kill 0`
# signals the whole process group — which on a remote shell takes the session
# down with it.
trap 'kill "$WESTON_PID" 2>/dev/null || true; [ -n "${HOST_PID:-}" ] && kill "$HOST_PID" 2>/dev/null; true' EXIT

for _ in $(seq 1 50); do
  [ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ] && break
  sleep 0.1
done

LOG=$(mktemp)
# The webview trace is what makes setContextMenuItems assertable: no automation
# can open a real context menu (GTK4 synthesises no pointer input), so the proof
# is the host reporting the tree it parsed and stored.
ND_WEBVIEW_TRACE=1 ND_SCRIPT=examples/webview-probe/main.tsx ./zig-out/bin/nd-hello >"$LOG" 2>&1 &
HOST_PID=$!

# 60s, not 20: the probe's first commit waits on the engine's own scheme
# registration, and a loaded machine has needed past 25s.
for _ in $(seq 1 600); do
  grep -q "ND_AUTOMATION_LISTENING" "$LOG" && grep -q "ND_COMMIT_APPLIED" "$LOG" && break
  sleep 0.1
done
grep -q "ND_AUTOMATION_LISTENING" "$LOG" || { echo "FAIL: no automation listener"; cat "$LOG"; exit 1; }
grep -q "ND_COMMIT_APPLIED" "$LOG" || { echo "FAIL: no commit applied"; cat "$LOG"; exit 1; }
# The engine loads lazily, on the app's first registerScheme / <webview>, so
# it is only observable after the probe has started.
for _ in $(seq 1 200); do
  grep -q "ND_WEBVIEW_ENGINE webkitgtk" "$LOG" && break
  sleep 0.1
done
grep -q "ND_WEBVIEW_ENGINE webkitgtk" "$LOG" || { echo "FAIL: webkitgtk engine did not load"; tail -40 "$LOG"; exit 1; }
SOCK=$(grep -m1 "ND_AUTOMATION_LISTENING" "$LOG" | sed 's/.*path=//')

ND_AUTOMATION_SOCKET="$SOCK" ND_SHOT_PATH="$XDG_RUNTIME_DIR/webview-probe.png" \
  bun scripts/webview-drive.ts >"$XDG_RUNTIME_DIR/drive.log" 2>&1 \
  || { echo "FAIL: driver"; cat "$XDG_RUNTIME_DIR/drive.log"; tail -80 "$LOG"; exit 1; }
cat "$XDG_RUNTIME_DIR/drive.log"
grep -q "ND_WEBVIEW2_OK" "$XDG_RUNTIME_DIR/drive.log" || { echo "FAIL: driver did not report success"; exit 1; }
grep -qE "setContextMenuItems node=[0-9]+ items=4" "$LOG" || {
  echo "FAIL: the host never stored the probe's context-menu items"
  grep -E "ND_WV .*(setContextMenuItems|ND_WARN)" "$LOG" | tail -20
  exit 1
}
echo "ND_WEBVIEW_CTXMENU_OK $(grep -m1 -oE "setContextMenuItems node=[0-9]+ items=4" "$LOG")"
# The redirect-echo invariant, from the host's own side. `setUrl` is traced
# only when a load was actually issued, and nothing but the echo check ever
# names /echo-committed, so a second line here IS the load storm: the app asked
# for the page the view was already on while a redirect was in flight, and the
# host obliged.
ECHO_LOADS=$(grep -cE "setUrl node=[0-9]+ url=.*/echo-committed" "$LOG" || true)
[ "$ECHO_LOADS" = "1" ] || {
  echo "FAIL: the committed page was loaded $ECHO_LOADS times, want 1"
  grep -E "ND_WV .*(setUrl|load node)" "$LOG" | tail -30
  exit 1
}
echo "ND_WEBVIEW_ECHO_OK 1 load for the committed page"
[ -s "$XDG_RUNTIME_DIR/webview-probe.png" ] || { echo "FAIL: empty png"; exit 1; }
file "$XDG_RUNTIME_DIR/webview-probe.png" | grep -q "PNG image" || { echo "FAIL: not a png"; exit 1; }

kill -TERM "$HOST_PID"; wait "$HOST_PID" 2>/dev/null || true
HOST_PID=""

echo "headless webview: OK (browser/extension surface verified, screenshot at $XDG_RUNTIME_DIR/webview-probe.png)"
