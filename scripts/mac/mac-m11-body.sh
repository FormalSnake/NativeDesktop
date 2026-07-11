#!/usr/bin/env bash
set -euo pipefail
# scripts/mac/mac-m11-body.sh — the M11 native-chrome acceptance body. Runs
# ON a Mac (the local clone, the ~/nd ssh clone, or a macos CI runner):
# builds libnd + the Swift shell, launches examples/notes under
# NATIVE_AUTOMATION, and requires the full notes round-trip (ND_NOTES_OK)
# AND the chrome assertions (ND_NAVCHROME_OK: splitview role=group,
# headerbar role=toolbar — the headerbar mounts as a real unified NSToolbar,
# the splitview as an NSSplitView with a vibrancy sidebar).
# Invoked directly on Darwin, or over ssh by scripts/mac/mac-m11.sh.
cd "$(dirname "$0")/../.."
ROOT="$(pwd -P)"
export PATH="/etc/profiles/per-user/kyandesutter/bin:/opt/homebrew/bin:$PATH"

zig build libnd -Dbackend=abi >/dev/null 2>&1
# zig's archiver emits members Apple's ld rejects ("not 8-byte aligned") and
# extracts them 0-permission; repack with the system ar/libtool before the
# Swift link (same recipe as scripts/mac/mac-run.sh).
workdir="$(mktemp -d)"
( cd "$workdir" && ar x "$ROOT/zig-out/lib/libnd.a" && chmod 644 *.o && libtool -static -o "$ROOT/zig-out/lib/libnd.a" *.o )
rm -rf "$workdir"
# env -u: nix devshells leak an SDKROOT that breaks the system Swift toolchain
# (harmless no-ops when the vars are unset).
( cd swift && env -u SDKROOT -u DEVELOPER_DIR swift build -c release >/dev/null 2>&1 )

pkill -f 'swift/.build/release/NDShell' 2>/dev/null || true
LOG=/tmp/nd-m11-notes.log
DRIVE_LOG=/tmp/nd-m11-drive.log
rm -f "$LOG" "$DRIVE_LOG" /tmp/notes-baseline.png /tmp/notes-final.png
ND_SCRIPT=examples/notes/main.tsx NATIVE_AUTOMATION=1 swift/.build/release/NDShell >"$LOG" 2>&1 &
PID=$!
for _ in $(seq 1 120); do
  grep -q ND_AUTOMATION_LISTENING "$LOG" && grep -q ND_COMMIT_APPLIED "$LOG" && break
  sleep 0.1
done
grep -q ND_AUTOMATION_LISTENING "$LOG" || { echo "FAIL: no automation listener"; cat "$LOG"; kill "$PID" 2>/dev/null; exit 1; }
grep -q ND_COMMIT_APPLIED "$LOG" || { echo "FAIL: no commit applied"; cat "$LOG"; kill "$PID" 2>/dev/null; exit 1; }
SOCK=$(grep -m1 ND_AUTOMATION_LISTENING "$LOG" | sed 's/.*path=//')

ND_AUTOMATION_SOCKET="$SOCK" ND_SHOT_DIR=/tmp bun scripts/notes-drive.ts >"$DRIVE_LOG" 2>&1 \
  || { echo "FAIL: notes drive"; cat "$DRIVE_LOG"; cat "$LOG"; kill "$PID" 2>/dev/null; exit 1; }
cat "$DRIVE_LOG"
grep -q ND_NOTES_OK "$DRIVE_LOG" || { echo "FAIL: no ND_NOTES_OK"; kill "$PID" 2>/dev/null; exit 1; }
grep -q ND_NAVCHROME_OK "$DRIVE_LOG" || { echo "FAIL: native chrome not present"; kill "$PID" 2>/dev/null; exit 1; }
file /tmp/notes-baseline.png | grep -q "PNG image" || { echo "FAIL: baseline not a png"; kill "$PID" 2>/dev/null; exit 1; }
file /tmp/notes-final.png | grep -q "PNG image" || { echo "FAIL: final not a png"; kill "$PID" 2>/dev/null; exit 1; }
kill -TERM "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
echo "MAC_M11_OK notes native chrome verified"
