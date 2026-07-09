#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
export WAYLAND_DISPLAY=nd-headless-m5b
export GSK_RENDERER=cairo
export GDK_BACKEND=wayland
export ND_SCRIPT=examples/gallery/main.tsx
export NATIVE_AUTOMATION=1

weston --backend=headless --socket="$WAYLAND_DISPLAY" --idle-time=0 &
WESTON_PID=$!
trap 'kill "$WESTON_PID" 2>/dev/null || true; kill "$HOST_PID" 2>/dev/null || true' EXIT

for _ in $(seq 1 50); do
  [ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ] && break
  sleep 0.1
done

LOG=$(mktemp)
./zig-out/bin/nd-hello >"$LOG" 2>&1 &
HOST_PID=$!

for _ in $(seq 1 80); do
  grep -q "ND_AUTOMATION_LISTENING" "$LOG" && grep -q "ND_COMMIT_APPLIED" "$LOG" && break
  sleep 0.1
done
grep -q "ND_AUTOMATION_LISTENING" "$LOG" || { echo "FAIL: no automation listener"; cat "$LOG"; exit 1; }
grep -q "ND_COMMIT_APPLIED" "$LOG" || { echo "FAIL: no commit applied"; cat "$LOG"; exit 1; }
# WebView stub must announce itself at create.
grep -q "ND_WARN WebView is a v1 stub" "$LOG" || { echo "FAIL: webview stub marker missing"; cat "$LOG"; exit 1; }
SOCK=$(grep -m1 "ND_AUTOMATION_LISTENING" "$LOG" | sed 's/.*path=//')
export ND_AUTOMATION_SOCKET="$SOCK"

ND_SHOT_PATH="$XDG_RUNTIME_DIR/m5b-shot.png" bun scripts/m5b-drive.ts >"$XDG_RUNTIME_DIR/drive.log" 2>&1 \
  || { echo "FAIL: driver"; cat "$XDG_RUNTIME_DIR/drive.log"; cat "$LOG"; exit 1; }
cat "$XDG_RUNTIME_DIR/drive.log"
grep -q "M5B_DRIVE_OK" "$XDG_RUNTIME_DIR/drive.log" || { echo "FAIL: driver did not report success"; exit 1; }
[ -s "$XDG_RUNTIME_DIR/m5b-shot.png" ] || { echo "FAIL: empty png"; exit 1; }
file "$XDG_RUNTIME_DIR/m5b-shot.png" | grep -q "PNG image" || { echo "FAIL: not a png"; exit 1; }

kill -TERM "$HOST_PID"; wait "$HOST_PID" 2>/dev/null || true
echo "headless m5b: OK (gallery driven: setValue/type/scroll + controlled round-trips)"
