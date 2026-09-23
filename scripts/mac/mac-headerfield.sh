#!/usr/bin/env bash
set -euo pipefail
# scripts/mac/mac-headerfield.sh -- the AppKit half of the header-bar field
# gate. Builds libnd + the Swift shell, runs examples/headerfield under the
# automation socket and drives it with scripts/headerfield-drive.ts, the same
# script the GTK leg runs.
#
# What it proves on AppKit: the header's search field takes the whole run
# between the toolbar's leading and trailing items at three window widths, the
# leading icon inside the field delivers its event, a popover that named the
# icon slot points at the icon rather than at the middle of the field, and a
# box's `font` style reaches the header button under it. With ND_NDSHOT (or a
# built tools/ndshot) it also captures the screen region with the popover up.
# Marker: MAC_HEADERFIELD_OK.
cd "$(dirname "$0")/../.."
ROOT="$(pwd -P)"
export PATH="/etc/profiles/per-user/kyandesutter/bin:/opt/homebrew/bin:$PATH"

zig build libnd -Dbackend=abi >/dev/null 2>&1
# zig's archiver emits members Apple's ld rejects ("not 8-byte aligned") and
# extracts them 0-permission; repack with the system ar/libtool before the
# Swift link (same recipe as scripts/mac/mac-m11-body.sh).
workdir="$(mktemp -d)"
( cd "$workdir" && ar x "$ROOT/zig-out/lib/libnd.a" && chmod 644 *.o && libtool -static -o "$ROOT/zig-out/lib/libnd.a" *.o )
rm -rf "$workdir"
( cd swift && env -u SDKROOT -u DEVELOPER_DIR swift build -c release >/dev/null 2>&1 )

NDSHOT="${ND_NDSHOT:-$ROOT/tools/ndshot/bin/ndshot}"
if [ -x "$NDSHOT" ]; then export ND_NDSHOT="$NDSHOT"; else unset ND_NDSHOT; echo "ND_WARN no ndshot at $NDSHOT; the region capture is skipped"; fi

LOG=/tmp/nd-headerfield-host.log
DRIVE_LOG=/tmp/nd-headerfield-drive.log
rm -f "$LOG" "$DRIVE_LOG"
ND_SCRIPT=examples/headerfield/main.tsx NATIVE_AUTOMATION=1 "$ROOT/swift/.build/release/NDShell" >"$LOG" 2>&1 &
PID=$!
trap 'kill "$PID" 2>/dev/null || true' EXIT
for _ in $(seq 1 200); do
  grep -q ND_AUTOMATION_LISTENING "$LOG" && grep -q ND_COMMIT_APPLIED "$LOG" && break
  sleep 0.1
done
grep -q ND_AUTOMATION_LISTENING "$LOG" || { echo "FAIL: no automation listener"; cat "$LOG"; exit 1; }
SOCK=$(grep -m1 ND_AUTOMATION_LISTENING "$LOG" | sed 's/.*path=//')

ND_HOST_PID="$PID" ND_BACKEND=appkit ND_AUTOMATION_SOCKET="$SOCK" ND_SHOT_PATH="${ND_SHOT_PATH:-/tmp/nd-headerfield-mac.png}" \
  ND_REGION_SHOT_PATH="${ND_REGION_SHOT_PATH:-/tmp/nd-headerfield-mac-region.png}" \
  bun scripts/headerfield-drive.ts >"$DRIVE_LOG" 2>&1 || { echo "FAIL: driver"; cat "$DRIVE_LOG"; tail -30 "$LOG"; exit 1; }
cat "$DRIVE_LOG"
grep -q ND_HEADERFIELD_OK "$DRIVE_LOG" || { echo "FAIL: no ND_HEADERFIELD_OK"; exit 1; }
kill -TERM "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
echo "MAC_HEADERFIELD_OK the header field fills the toolbar run and its leading icon fires"
