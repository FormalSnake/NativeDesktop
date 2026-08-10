#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
export WAYLAND_DISPLAY=nd-headless-panes
export GSK_RENDERER=cairo GDK_BACKEND=wayland
export NATIVE_AUTOMATION=1
export ND_SCRIPT="$(pwd)/examples/panes/main.tsx"
# The example passes ND_STORE_DIR as the store's dir override, so both host
# runs share one throwaway settings dir and run 2 proves the restore.
export ND_STORE_DIR="$(mktemp -d)"

weston --backend=headless --width=1280 --height=800 --socket="$WAYLAND_DISPLAY" --idle-time=0 &
WESTON_PID=$!
trap 'kill "$WESTON_PID" "$HOST_PID" 2>/dev/null || true' EXIT
for _ in $(seq 1 50); do [ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ] && break; sleep 0.1; done

run_host() {
  LOG=$(mktemp)
  ./zig-out/bin/nd-hello >"$LOG" 2>&1 &
  HOST_PID=$!
  for _ in $(seq 1 120); do
    grep -q ND_AUTOMATION_LISTENING "$LOG" && grep -q ND_COMMIT_APPLIED "$LOG" && break
    sleep 0.1
  done
  grep -q ND_AUTOMATION_LISTENING "$LOG" || { echo "FAIL: no automation listener"; cat "$LOG"; exit 1; }
  grep -q ND_COMMIT_APPLIED "$LOG" || { echo "FAIL: no commit applied"; cat "$LOG"; exit 1; }
  ND_AUTOMATION_SOCKET=$(grep -m1 ND_AUTOMATION_LISTENING "$LOG" | sed 's/.*path=//')
  export ND_AUTOMATION_SOCKET
}

# Run 1: build the split layout and persist it.
run_host
bun scripts/panes-drive.ts >"$XDG_RUNTIME_DIR/drive1.log" 2>&1 \
  || { echo "FAIL: build drive"; cat "$XDG_RUNTIME_DIR/drive1.log" "$LOG"; exit 1; }
cat "$XDG_RUNTIME_DIR/drive1.log"
grep -q ND_PANES_OK "$XDG_RUNTIME_DIR/drive1.log" || { echo "FAIL: no ND_PANES_OK"; exit 1; }
kill -TERM "$HOST_PID"
wait "$HOST_PID" 2>/dev/null || true

[ -s "$ND_STORE_DIR/panes.json" ] || { echo "FAIL: panes.json was not persisted"; ls -la "$ND_STORE_DIR"; exit 1; }

# Run 2: a fresh host against the same store dir must restore the layout.
run_host
bun scripts/panes-drive.ts --restore >"$XDG_RUNTIME_DIR/drive2.log" 2>&1 \
  || { echo "FAIL: restore drive"; cat "$XDG_RUNTIME_DIR/drive2.log" "$LOG"; exit 1; }
cat "$XDG_RUNTIME_DIR/drive2.log"
grep -q ND_PANES_RESTORE_OK "$XDG_RUNTIME_DIR/drive2.log" || { echo "FAIL: no ND_PANES_RESTORE_OK"; exit 1; }
kill -TERM "$HOST_PID"
wait "$HOST_PID" 2>/dev/null || true

echo "headless panes: OK (split/close/ratio round-trip + store restore across host runs)"
