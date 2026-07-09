#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
export WAYLAND_DISPLAY=nd-headless-m4
export GSK_RENDERER=cairo
export GDK_BACKEND=wayland
export ND_SCRIPT=examples/counter/main.tsx
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

# Wait for the automation listener + the react handshake + the first commit (so widgets exist).
for _ in $(seq 1 80); do
  grep -q "ND_AUTOMATION_LISTENING" "$LOG" && grep -q "ND_COMMIT_APPLIED" "$LOG" && break
  sleep 0.1
done
grep -q "ND_AUTOMATION_LISTENING" "$LOG" || { echo "FAIL: no automation listener"; cat "$LOG"; exit 1; }
grep -q "ND_COMMIT_APPLIED" "$LOG" || { echo "FAIL: no commit applied"; cat "$LOG"; exit 1; }
SOCK=$(grep -m1 "ND_AUTOMATION_LISTENING" "$LOG" | sed 's/.*path=//')
export ND_AUTOMATION_SOCKET="$SOCK"

# Drive the app via the socket directly.
ND_SHOT_PATH="$XDG_RUNTIME_DIR/m4-shot.png" bun scripts/m4-drive.ts >"$XDG_RUNTIME_DIR/drive.log" 2>&1 \
  || { echo "FAIL: driver"; cat "$XDG_RUNTIME_DIR/drive.log"; cat "$LOG"; exit 1; }
cat "$XDG_RUNTIME_DIR/drive.log"
grep -q "M4_DRIVE_OK" "$XDG_RUNTIME_DIR/drive.log" || { echo "FAIL: driver did not report success"; exit 1; }
[ -s "$XDG_RUNTIME_DIR/m4-shot.png" ] || { echo "FAIL: empty png"; exit 1; }
file "$XDG_RUNTIME_DIR/m4-shot.png" | grep -q "PNG image" || { echo "FAIL: not a png"; exit 1; }

# ---- D11 SLO: stall the bun child, automation must still answer within 5s ----
# The bun child is the host's direct child process — find it via the process
# tree (pgrep -P), never by matching bun processes machine-wide (kill9-test.sh pattern).
BUN_PID=$(pgrep -P "$HOST_PID" -f "bun" | head -1)
[ -n "$BUN_PID" ] || { echo "FAIL: no bun child for SLO test"; exit 1; }
kill -STOP "$BUN_PID"

SLO_PNG="$XDG_RUNTIME_DIR/m4-slo.png"
SLO_STATUS=0
timeout 5 bash -c "ND_AUTOMATION_SOCKET='$SOCK' ND_SHOT_PATH='$SLO_PNG' bun scripts/m4-drive.ts --slo" >"$XDG_RUNTIME_DIR/slo.log" 2>&1 || SLO_STATUS=$?
kill -CONT "$BUN_PID"
[ "$SLO_STATUS" -eq 0 ] || { echo "FAIL: automation did not answer within 5s while child stalled"; cat "$XDG_RUNTIME_DIR/slo.log"; cat "$LOG"; exit 1; }
cat "$XDG_RUNTIME_DIR/slo.log"
grep -q "M4_SLO_OK" "$XDG_RUNTIME_DIR/slo.log" || { echo "FAIL: SLO driver did not report success"; exit 1; }
[ -s "$SLO_PNG" ] || { echo "FAIL: empty slo png"; exit 1; }

kill -TERM "$HOST_PID"; wait "$HOST_PID" 2>/dev/null || true
echo "headless m4: OK (drove counter, screenshot written, D11 SLO under stall)"
