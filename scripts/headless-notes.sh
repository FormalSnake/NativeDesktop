#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
export WAYLAND_DISPLAY=wl-notes-0
export GSK_RENDERER=cairo
export GDK_BACKEND=wayland
export NATIVE_AUTOMATION=1

weston --backend=headless --socket="$WAYLAND_DISPLAY" --idle-time=0 &
WESTON_PID=$!
trap 'kill "$WESTON_PID" 2>/dev/null || true; kill "$HOST_PID" 2>/dev/null || true' EXIT

for _ in $(seq 1 50); do
  [ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ] && break
  sleep 0.1
done

LOG=$(mktemp)
ND_SCRIPT=examples/notes/main.tsx ./zig-out/bin/nd-hello >"$LOG" 2>&1 &
HOST_PID=$!

for _ in $(seq 1 120); do
  grep -q "ND_AUTOMATION_LISTENING" "$LOG" && grep -q "ND_COMMIT_APPLIED" "$LOG" && break
  sleep 0.1
done
grep -q "ND_AUTOMATION_LISTENING" "$LOG" || { echo "FAIL: no automation listener"; cat "$LOG"; exit 1; }
grep -q "ND_COMMIT_APPLIED" "$LOG" || { echo "FAIL: no commit applied"; cat "$LOG"; exit 1; }
SOCK=$(grep -m1 "ND_AUTOMATION_LISTENING" "$LOG" | sed 's/.*path=//')

ND_AUTOMATION_SOCKET="$SOCK" ND_SHOT_DIR="$XDG_RUNTIME_DIR" bun scripts/notes-drive.ts >"$XDG_RUNTIME_DIR/drive.log" 2>&1 \
  || { echo "FAIL: driver"; cat "$XDG_RUNTIME_DIR/drive.log"; cat "$LOG"; exit 1; }
cat "$XDG_RUNTIME_DIR/drive.log"
grep -q "ND_NOTES_OK" "$XDG_RUNTIME_DIR/drive.log" || { echo "FAIL: driver did not report success"; exit 1; }
grep -q "ND_NAVCHROME_OK" "$XDG_RUNTIME_DIR/drive.log" || { echo "FAIL: native chrome not present"; exit 1; }

for shot in "$XDG_RUNTIME_DIR/notes-baseline.png" "$XDG_RUNTIME_DIR/notes-final.png"; do
  [ -s "$shot" ] || { echo "FAIL: empty png $shot"; exit 1; }
  file "$shot" | grep -q "PNG image" || { echo "FAIL: not a png $shot"; exit 1; }
done

echo "headless notes: OK (create/edit/search/pin/delete round-trip + screenshots)"
