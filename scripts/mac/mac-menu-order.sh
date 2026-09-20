#!/usr/bin/env bash
set -euo pipefail
# scripts/mac/mac-menu-order.sh: the AppKit half of the menu-order gate. Runs
# ON a Mac: builds libnd + the Swift shell, then runs
# examples/notes/menu-order-probe.tsx under NATIVE_AUTOMATION and drives it
# with scripts/menu-order-drive.ts (the Linux peer is
# scripts/headless-menu-order.sh). The drive reads NSApp.mainMenu and the
# NSComboButton's menu back through the menuModel RPC and compares them to
# React's order after a move, a middle remove and a middle insert.
cd "$(dirname "$0")/../.."
ROOT="$(pwd -P)"
export PATH="/etc/profiles/per-user/kyandesutter/bin:/opt/homebrew/bin:$PATH"

zig build libnd -Dbackend=abi >/dev/null 2>&1
# Repack zig's archive for Apple's ld (same recipe as mac-gestures.sh).
workdir="$(mktemp -d)"
( cd "$workdir" && ar x "$ROOT/zig-out/lib/libnd.a" && chmod 644 *.o && libtool -static -o "$ROOT/zig-out/lib/libnd.a" *.o )
rm -rf "$workdir"
( cd swift && env -u SDKROOT -u DEVELOPER_DIR swift build -c release >/dev/null 2>&1 )

LOG=/tmp/nd-menuorder.log
DRIVE_LOG=/tmp/nd-menuorder-drive.log
pkill -f 'swift/.build/release/NDShell' 2>/dev/null || true
rm -f "$LOG" "$DRIVE_LOG"
ND_SCRIPT=examples/notes/menu-order-probe.tsx NATIVE_AUTOMATION=1 swift/.build/release/NDShell >"$LOG" 2>&1 &
PID=$!
trap '[ -n "${PID:-}" ] && kill "$PID" 2>/dev/null; true' EXIT
for _ in $(seq 1 120); do
  grep -q ND_AUTOMATION_LISTENING "$LOG" 2>/dev/null && grep -q ND_COMMIT_APPLIED "$LOG" && break
  sleep 0.1
done
grep -q ND_AUTOMATION_LISTENING "$LOG" || { echo "FAIL: no automation listener"; cat "$LOG"; exit 1; }
SOCK=$(grep -m1 ND_AUTOMATION_LISTENING "$LOG" | sed 's/.*path=//')

ND_AUTOMATION_SOCKET="$SOCK" bun scripts/menu-order-drive.ts >"$DRIVE_LOG" 2>&1 \
  || { echo "FAIL: menu-order drive"; cat "$DRIVE_LOG"; tail -40 "$LOG"; exit 1; }
cat "$DRIVE_LOG"
grep -q ND_MENU_ORDER_OK "$DRIVE_LOG" || { echo "FAIL: no ND_MENU_ORDER_OK"; exit 1; }

kill -TERM "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
PID=""
echo "MAC_MENU_ORDER_OK the native menu matched React after a move, a middle remove and a middle insert"
