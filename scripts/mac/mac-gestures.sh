#!/usr/bin/env bash
set -euo pipefail
# scripts/mac/mac-gestures.sh — the M16 agentic-input acceptance body. Runs
# ON a Mac: builds libnd + the Swift shell, then
#   leg 1: examples/gestures under NATIVE_AUTOMATION, scripts/gestures-drive.ts
#          (a11y tree fields, coordinate pointer, slider drag through the real
#          NSSlider tracking loop, table double-click, rightClick/hover, keys
#          typing + cmd+a chord, getTree window param) -> ND_GESTURES_OK
#   leg 2: examples/notes, scripts/keys-menu-drive.ts (cmd+n key equivalent
#          runs File > New Note) -> ND_KEYS_MENU_OK
#   leg 3: examples/locators, scripts/locator-drive.ts (focus/press,
#          scrollIntoView on a clipped row, node-sized snapshotNode,
#          setWindowFrame, check/uncheck, selectOption by label)
#          -> ND_LOCATOR_OK
cd "$(dirname "$0")/../.."
ROOT="$(pwd -P)"
export PATH="/etc/profiles/per-user/kyandesutter/bin:/opt/homebrew/bin:$PATH"

zig build libnd -Dbackend=abi >/dev/null 2>&1
# Repack zig's archive for Apple's ld (same recipe as mac-m11-body.sh).
workdir="$(mktemp -d)"
( cd "$workdir" && ar x "$ROOT/zig-out/lib/libnd.a" && chmod 644 *.o && libtool -static -o "$ROOT/zig-out/lib/libnd.a" *.o )
rm -rf "$workdir"
( cd swift && env -u SDKROOT -u DEVELOPER_DIR swift build -c release >/dev/null 2>&1 )

run_leg() {
  local script="$1" drive="$2" log="$3" drive_log="$4" marker="$5"
  pkill -f 'swift/.build/release/NDShell' 2>/dev/null || true
  rm -f "$log" "$drive_log"
  ND_SCRIPT="$script" NATIVE_AUTOMATION=1 swift/.build/release/NDShell >"$log" 2>&1 &
  local pid=$!
  for _ in $(seq 1 120); do
    grep -q ND_AUTOMATION_LISTENING "$log" 2>/dev/null && grep -q ND_COMMIT_APPLIED "$log" && break
    sleep 0.1
  done
  grep -q ND_AUTOMATION_LISTENING "$log" || { echo "FAIL: no automation listener ($script)"; cat "$log"; kill "$pid" 2>/dev/null; exit 1; }
  local sock
  sock=$(grep -m1 ND_AUTOMATION_LISTENING "$log" | sed 's/.*path=//')
  ND_AUTOMATION_SOCKET="$sock" bun "$drive" >"$drive_log" 2>&1 \
    || { echo "FAIL: $drive"; cat "$drive_log"; cat "$log"; kill "$pid" 2>/dev/null; exit 1; }
  cat "$drive_log"
  grep -q "$marker" "$drive_log" || { echo "FAIL: no $marker"; kill "$pid" 2>/dev/null; exit 1; }
  kill -TERM "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
}

run_leg examples/gestures/main.tsx scripts/gestures-drive.ts /tmp/nd-gestures.log /tmp/nd-gestures-drive.log ND_GESTURES_OK
run_leg examples/notes/main.tsx scripts/keys-menu-drive.ts /tmp/nd-keysmenu.log /tmp/nd-keysmenu-drive.log ND_KEYS_MENU_OK
run_leg examples/locators/main.tsx scripts/locator-drive.ts /tmp/nd-locator.log /tmp/nd-locator-drive.log ND_LOCATOR_OK

echo "MAC_GESTURES_OK agentic input + a11y tree + locators verified"
