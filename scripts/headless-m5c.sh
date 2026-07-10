#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
export WAYLAND_DISPLAY=nd-headless-m5c
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

# Phase A: styled tab + 100k-row ListView gallery drive.
LOG=$(mktemp)
ND_SCRIPT=examples/gallery/main.tsx ./zig-out/bin/nd-hello >"$LOG" 2>&1 &
HOST_PID=$!

for _ in $(seq 1 120); do
  grep -q "ND_AUTOMATION_LISTENING" "$LOG" && grep -q "ND_COMMIT_APPLIED" "$LOG" && break
  sleep 0.1
done
grep -q "ND_AUTOMATION_LISTENING" "$LOG" || { echo "FAIL: no automation listener"; cat "$LOG"; exit 1; }
grep -q "ND_COMMIT_APPLIED" "$LOG" || { echo "FAIL: no commit applied"; cat "$LOG"; exit 1; }
SOCK=$(grep -m1 "ND_AUTOMATION_LISTENING" "$LOG" | sed 's/.*path=//')

ND_AUTOMATION_SOCKET="$SOCK" ND_SHOT_PATH="$XDG_RUNTIME_DIR/m5c-shot.png" bun scripts/m5c-drive.ts >"$XDG_RUNTIME_DIR/drive.log" 2>&1 \
  || { echo "FAIL: driver"; cat "$XDG_RUNTIME_DIR/drive.log"; cat "$LOG"; exit 1; }
cat "$XDG_RUNTIME_DIR/drive.log"
grep -q "M5C_DRIVE_OK" "$XDG_RUNTIME_DIR/drive.log" || { echo "FAIL: driver did not report success"; exit 1; }
[ -s "$XDG_RUNTIME_DIR/m5c-shot.png" ] || { echo "FAIL: empty png"; exit 1; }
file "$XDG_RUNTIME_DIR/m5c-shot.png" | grep -q "PNG image" || { echo "FAIL: not a png"; exit 1; }

kill -TERM "$HOST_PID"; wait "$HOST_PID" 2>/dev/null || true
HOST_PID=""

# Phase B: web-CSS rejection must crash loudly with the fix-it message. The
# Bun child crashes at mount (StyleError thrown by validateStyle -> the
# renderer's error handler -> process exits nonzero); the host process itself
# does not exit on its own, so bound the run with timeout and grep stderr
# (`|| true` — a nonzero/timeout exit here is expected, not a script failure).
BADLOG=$(mktemp)
ND_SCRIPT=examples/gallery/gallery-badstyle.tsx timeout 10s ./zig-out/bin/nd-hello >"$BADLOG" 2>&1 || true
grep -q 'Invalid style key "display" — GTK styling is not web CSS' "$BADLOG" || { echo "FAIL: no fix-it rejection"; cat "$BADLOG"; exit 1; }
grep -q 'docs/styling.md' "$BADLOG" || { echo "FAIL: fix-it missing docs pointer"; cat "$BADLOG"; exit 1; }

echo "headless m5c: OK (styling round-trip + web-CSS rejection + 100k ListView)"
