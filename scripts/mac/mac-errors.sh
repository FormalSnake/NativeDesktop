#!/usr/bin/env bash
set -euo pipefail
# scripts/mac/mac-errors.sh — the AppKit half of scripts/headless-errors.sh.
# Runs ON a Mac: builds libnd + the Swift shell, then drives examples/errors
# through one host process:
#   survive:       non-fatal rejection + boundary-caught render throw, app keeps
#                  running (>=2 ND_RUNTIME_ERROR_NONFATAL, no child exit, no
#                  overlay) -> ERRORS_SURVIVE_OK
#   fatal:         sync throw exits the child and paints the crash overlay
#                  -> ND_CHILD_EXITED + ND_OVERLAY_SHOWN
#   overlay-check: the overlay's nodes are IN the automation tree and carry the
#                  fatal message, not the earlier non-fatal one (the no-stash
#                  rule) -> ERRORS_OVERLAY_OK
#   restart:       the overlay's Restart button answers a semantic click, the
#                  app comes back and the overlay's nodes leave the tree
#                  -> ERRORS_RESTART_OK
cd "$(dirname "$0")/../.."
ROOT="$(pwd -P)"
export PATH="/etc/profiles/per-user/kyandesutter/bin:/opt/homebrew/bin:$PATH"

zig build libnd -Dbackend=abi >/dev/null 2>&1
# Repack zig's archive for Apple's ld (same recipe as mac-gestures.sh).
workdir="$(mktemp -d)"
( cd "$workdir" && ar x "$ROOT/zig-out/lib/libnd.a" && chmod 644 *.o && libtool -static -o "$ROOT/zig-out/lib/libnd.a" *.o )
rm -rf "$workdir"
( cd swift && env -u SDKROOT -u DEVELOPER_DIR swift build -c release >/dev/null 2>&1 )

LOG=/tmp/nd-mac-errors.log
DRIVE=/tmp/nd-mac-errors-drive.log
pkill -f 'swift/.build/release/NDShell' 2>/dev/null || true
rm -f "$LOG" "$DRIVE"

ND_SCRIPT="$ROOT/examples/errors/main.tsx" NATIVE_AUTOMATION=1 ND_DEV=1 \
  swift/.build/release/NDShell >"$LOG" 2>&1 &
PID=$!
trap 'kill "$PID" 2>/dev/null || true' EXIT
for _ in $(seq 1 150); do
  grep -q ND_AUTOMATION_LISTENING "$LOG" 2>/dev/null && grep -q ND_COMMIT_APPLIED "$LOG" && break
  sleep 0.1
done
grep -q ND_AUTOMATION_LISTENING "$LOG" || { echo "FAIL: no automation listener"; cat "$LOG"; exit 1; }
grep -q ND_COMMIT_APPLIED "$LOG" || { echo "FAIL: no commit applied"; cat "$LOG"; exit 1; }
export ND_AUTOMATION_SOCKET="$(grep -m1 ND_AUTOMATION_LISTENING "$LOG" | sed 's/.*path=//')"

leg() {
  local flag="$1" marker="$2"
  bun scripts/errors-drive.ts "$flag" >"$DRIVE" 2>&1 \
    || { echo "FAIL: errors-drive $flag"; cat "$DRIVE" "$LOG"; exit 1; }
  cat "$DRIVE"
  grep -q "$marker" "$DRIVE" || { echo "FAIL: no $marker"; cat "$LOG"; exit 1; }
}

# ---- survive leg: non-fatal rejection + boundary catch; app keeps running ----
EXITED_BEFORE=$(grep -c ND_CHILD_EXITED "$LOG" || true)
leg --survive ERRORS_SURVIVE_OK

# One report per non-fatal error: the rejection and the boundary-caught throw.
for _ in $(seq 1 50); do [ "$(grep -c ND_RUNTIME_ERROR_NONFATAL "$LOG" || true)" -ge 2 ] && break; sleep 0.1; done
NONFATAL=$(grep -c ND_RUNTIME_ERROR_NONFATAL "$LOG" || true)
[ "$NONFATAL" -ge 2 ] || { echo "FAIL: expected >=2 ND_RUNTIME_ERROR_NONFATAL, got $NONFATAL"; cat "$LOG"; exit 1; }

EXITED_AFTER=$(grep -c ND_CHILD_EXITED "$LOG" || true)
[ "$EXITED_BEFORE" = "$EXITED_AFTER" ] || { echo "FAIL: child exited during survive leg"; cat "$LOG"; exit 1; }
grep -q ND_OVERLAY_SHOWN "$LOG" && { echo "FAIL: overlay shown for a non-fatal error"; cat "$LOG"; exit 1; }

# ---- fatal leg: sync throw exits the child and paints the overlay ----
leg --fatal ERRORS_FATAL_CLICKED
for _ in $(seq 1 80); do grep -q ND_CHILD_EXITED "$LOG" && grep -q ND_OVERLAY_SHOWN "$LOG" && break; sleep 0.1; done
grep -q ND_CHILD_EXITED "$LOG" || { echo "FAIL: child did not exit on sync throw"; cat "$LOG"; exit 1; }
grep -q ND_OVERLAY_SHOWN "$LOG" || { echo "FAIL: no overlay after fatal error"; cat "$LOG"; exit 1; }

# ---- the overlay is a real part of the automation tree, showing the fatal
# ---- message rather than the earlier non-fatal one ----
leg --overlay-check ERRORS_OVERLAY_OK

# ---- its Restart button takes a semantic click, and the chrome leaves the
# ---- tree once the respawned app has mounted ----
leg --restart ERRORS_RESTART_OK

kill -TERM "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
echo "MAC_ERRORS_OK crash overlay visible + clickable through getTree"
