#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
export WAYLAND_DISPLAY=nd-headless-errors
export GSK_RENDERER=cairo GDK_BACKEND=wayland
export NATIVE_AUTOMATION=1 ND_DEV=1
export ND_SCRIPT="$(pwd)/examples/errors/main.tsx"

weston --backend=headless --socket="$WAYLAND_DISPLAY" --idle-time=0 &
WESTON_PID=$!
trap 'kill "$WESTON_PID" "$HOST_PID" 2>/dev/null || true' EXIT
for _ in $(seq 1 50); do [ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ] && break; sleep 0.1; done

LOG=$(mktemp)
./zig-out/bin/nd-hello >"$LOG" 2>&1 &
HOST_PID=$!
for _ in $(seq 1 80); do grep -q ND_AUTOMATION_LISTENING "$LOG" && grep -q ND_COMMIT_APPLIED "$LOG" && break; sleep 0.1; done
grep -q ND_AUTOMATION_LISTENING "$LOG" || { echo "FAIL: no automation listener"; cat "$LOG"; exit 1; }
grep -q ND_COMMIT_APPLIED "$LOG" || { echo "FAIL: no commit applied"; cat "$LOG"; exit 1; }
SOCK=$(grep -m1 ND_AUTOMATION_LISTENING "$LOG" | sed 's/.*path=//')
export ND_AUTOMATION_SOCKET="$SOCK"

# ---- survive leg: non-fatal rejection + boundary catch; app keeps running ----
EXITED_BEFORE=$(grep -c ND_CHILD_EXITED "$LOG" || true)
bun scripts/errors-drive.ts --survive >"$XDG_RUNTIME_DIR/survive.log" 2>&1 || { echo "FAIL survive drive"; cat "$XDG_RUNTIME_DIR/survive.log" "$LOG"; exit 1; }
cat "$XDG_RUNTIME_DIR/survive.log"
grep -q ERRORS_SURVIVE_OK "$XDG_RUNTIME_DIR/survive.log" || { echo "FAIL: survive marker"; exit 1; }

# One report per non-fatal error: the rejection and the boundary-caught throw.
for _ in $(seq 1 50); do [ "$(grep -c ND_RUNTIME_ERROR_NONFATAL "$LOG" || true)" -ge 2 ] && break; sleep 0.1; done
NONFATAL=$(grep -c ND_RUNTIME_ERROR_NONFATAL "$LOG" || true)
[ "$NONFATAL" -ge 2 ] || { echo "FAIL: expected >=2 ND_RUNTIME_ERROR_NONFATAL, got $NONFATAL"; cat "$LOG"; exit 1; }

EXITED_AFTER=$(grep -c ND_CHILD_EXITED "$LOG" || true)
[ "$EXITED_BEFORE" = "$EXITED_AFTER" ] || { echo "FAIL: child exited during survive leg (before=$EXITED_BEFORE after=$EXITED_AFTER)"; cat "$LOG"; exit 1; }
# The overlay must never paint for a survived error (m8's crash leg greps for
# this marker; here its absence is the assertion).
grep -q ND_OVERLAY_SHOWN "$LOG" && { echo "FAIL: overlay shown for a non-fatal error"; cat "$LOG"; exit 1; }

# ---- fatal leg: sync throw exits the child and paints the overlay ----
bun scripts/errors-drive.ts --fatal >"$XDG_RUNTIME_DIR/fatal.log" 2>&1 || { echo "FAIL fatal drive"; cat "$XDG_RUNTIME_DIR/fatal.log" "$LOG"; exit 1; }
cat "$XDG_RUNTIME_DIR/fatal.log"
grep -q ERRORS_FATAL_CLICKED "$XDG_RUNTIME_DIR/fatal.log" || { echo "FAIL: fatal click marker"; exit 1; }

for _ in $(seq 1 50); do grep -q ND_CHILD_EXITED "$LOG" && grep -q ND_OVERLAY_SHOWN "$LOG" && break; sleep 0.1; done
grep -q ND_CHILD_EXITED "$LOG" || { echo "FAIL: child did not exit on sync throw"; cat "$LOG"; exit 1; }
grep -q ND_OVERLAY_SHOWN "$LOG" || { echo "FAIL: no overlay after fatal error"; cat "$LOG"; exit 1; }

# The overlay must show the fatal message, not the earlier non-fatal one
# (regression guard for the host's no-stash rule).
bun scripts/errors-drive.ts --overlay-check >"$XDG_RUNTIME_DIR/overlay.log" 2>&1 || { echo "FAIL overlay check"; cat "$XDG_RUNTIME_DIR/overlay.log" "$LOG"; exit 1; }
cat "$XDG_RUNTIME_DIR/overlay.log"
grep -q ERRORS_OVERLAY_OK "$XDG_RUNTIME_DIR/overlay.log" || { echo "FAIL: overlay marker"; exit 1; }

kill -TERM "$HOST_PID"; wait "$HOST_PID" 2>/dev/null || true
echo "ND_ERRORS_OK"
echo "headless errors: OK (non-fatal rejection survives + boundary catch + fatal overlay no-stash)"
