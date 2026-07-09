#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
export WAYLAND_DISPLAY=nd-headless-kill9
export GSK_RENDERER=cairo
export GDK_BACKEND=wayland

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

# Wait for the child to connect.
for _ in $(seq 1 50); do
  grep -q "ND_CHILD_CONNECTED" "$LOG" && break
  sleep 0.1
done
grep -q "ND_CHILD_CONNECTED" "$LOG" || { echo "FAIL: child never connected"; cat "$LOG"; exit 1; }

# The bun child is the host's direct child process — find it via the process
# tree (pgrep -P), never by matching bun processes machine-wide.
BUN_PID=$(pgrep -P "$HOST_PID" -f "bun" | head -1)
[ -n "$BUN_PID" ] || { echo "FAIL: no bun pid"; cat "$LOG"; exit 1; }
kill -9 "$BUN_PID"

# Host must observe the exit and stay alive.
for _ in $(seq 1 30); do
  grep -q "ND_CHILD_EXITED" "$LOG" && break
  sleep 0.1
done
grep -q "ND_CHILD_EXITED" "$LOG" || { echo "FAIL: host did not report child exit"; cat "$LOG"; exit 1; }

sleep 3
kill -0 "$HOST_PID" 2>/dev/null || { echo "FAIL: host died with the child"; cat "$LOG"; exit 1; }

# Clean shutdown on SIGTERM.
kill -TERM "$HOST_PID"
wait "$HOST_PID" 2>/dev/null || true
echo "kill9: OK — window survived child death"
