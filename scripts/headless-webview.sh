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
ND_SCRIPT=examples/webview-probe/main.tsx ./zig-out/bin/nd-hello >"$LOG" 2>&1 &
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
[ -s "$XDG_RUNTIME_DIR/webview-probe.png" ] || { echo "FAIL: empty png"; exit 1; }
file "$XDG_RUNTIME_DIR/webview-probe.png" | grep -q "PNG image" || { echo "FAIL: not a png"; exit 1; }

kill -TERM "$HOST_PID"; wait "$HOST_PID" 2>/dev/null || true
HOST_PID=""

echo "headless webview: OK (browser/extension surface verified, screenshot at $XDG_RUNTIME_DIR/webview-probe.png)"
