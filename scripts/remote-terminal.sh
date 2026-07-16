#!/usr/bin/env bash
set -euo pipefail
# scripts/remote-terminal.sh [appkit|gtk] — end-to-end M3 remote-terminal drive.
# Starts the fake byte-plane server on an ephemeral port, launches the
# remote-terminal example on the chosen backend with NATIVE_AUTOMATION=1, and
# runs scripts/remote-terminal-drive.ts against it (asserts ATTACHED, the
# title/bell/exit events, the FLAG_RESET snapshot, and a non-empty screenshot).
#
# GTK builds/runs need the nix devshell (pkg-config + brew GTK); wrap the whole
# call in `nix develop --command`. AppKit needs only zig+swift on PATH.
cd "$(dirname "$0")/.."
BACKEND="${1:-appkit}"
HOST_PID=""
FAKE_PID=""
cleanup() { [ -n "$HOST_PID" ] && kill "$HOST_PID" 2>/dev/null || true; [ -n "$FAKE_PID" ] && kill "$FAKE_PID" 2>/dev/null || true; }
trap cleanup EXIT

FAKE_LOG=$(mktemp)
bun scripts/remote-terminal-fake-server.ts 0 >"$FAKE_LOG" 2>&1 &
FAKE_PID=$!
for _ in $(seq 1 50); do grep -q FAKE_SERVER_LISTENING "$FAKE_LOG" && break; sleep 0.1; done
grep -q FAKE_SERVER_LISTENING "$FAKE_LOG" || { echo "FAIL: fake server did not bind"; cat "$FAKE_LOG"; exit 1; }
PORT=$(grep -m1 FAKE_SERVER_LISTENING "$FAKE_LOG" | sed 's/.*port=//')
echo "fake server on port $PORT"

export ND_SCRIPT=examples/remote-terminal/main.tsx
export NATIVE_AUTOMATION=1
export ND_REMOTE_HOST=127.0.0.1
export ND_REMOTE_PORT="$PORT"
export ND_REMOTE_SESSION=sess-demo
export ND_REMOTE_TICKET=ticket-demo

HOST_LOG=$(mktemp)
if [ "$BACKEND" = gtk ]; then
  zig build >/dev/null 2>&1 || { echo "FAIL: zig build (needs nix devshell for GTK)"; exit 1; }
  ./zig-out/bin/nd-hello >"$HOST_LOG" 2>&1 &
else
  zig build libnd -Dbackend=abi >/dev/null 2>&1
  ( cd swift && swift build >/dev/null 2>&1 )
  swift/.build/debug/NDShell >"$HOST_LOG" 2>&1 &
fi
HOST_PID=$!

for _ in $(seq 1 150); do
  grep -q ND_AUTOMATION_LISTENING "$HOST_LOG" && grep -q ND_COMMIT_APPLIED "$HOST_LOG" && break
  sleep 0.1
done
grep -q ND_AUTOMATION_LISTENING "$HOST_LOG" || { echo "FAIL: no automation listener"; cat "$HOST_LOG"; exit 1; }
SOCK=$(grep -m1 ND_AUTOMATION_LISTENING "$HOST_LOG" | sed 's/.*path=//')

DRIVE_LOG=$(mktemp)
SHOT="/tmp/remote-terminal-$BACKEND.png"
ND_AUTOMATION_SOCKET="$SOCK" ND_SHOT_PATH="$SHOT" bun scripts/remote-terminal-drive.ts >"$DRIVE_LOG" 2>&1 \
  || { echo "FAIL: driver"; cat "$DRIVE_LOG"; echo "---- host log ----"; tail -30 "$HOST_LOG"; exit 1; }
cat "$DRIVE_LOG"
grep -q REMOTE_DRIVE_OK "$DRIVE_LOG" || { echo "FAIL: driver did not report success"; exit 1; }
file "$SHOT" | grep -q "PNG image" || { echo "FAIL: screenshot is not a PNG"; exit 1; }

kill -TERM "$HOST_PID" 2>/dev/null || true; wait "$HOST_PID" 2>/dev/null || true; HOST_PID=""
echo "REMOTE_TERMINAL_${BACKEND}: OK"
