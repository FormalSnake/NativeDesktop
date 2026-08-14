#!/usr/bin/env bash
# Generic weston-headless harness: boot a compositor, run one example under the
# automation socket, drive it with one script, and require a marker in the
# driver's output.
#
#   scripts/headless-run.sh <example-main.tsx> <drive.ts> <MARKER> [display-tag]
#
# The twelve headless-*.sh gates each hand-roll this same sequence; new legs
# (and scripts/browser-gate.sh) build on this instead of copying it again.
# Every run gets its OWN application id and wayland socket: GApplication is
# single-instance per id on the session bus, so two gates (or an agent's app
# and a gate) sharing `dev.nativedesktop.hello` make the second one exit with
# ND_ALREADY_RUNNING instead of starting.
set -euo pipefail
cd "$(dirname "$0")/.."

EXAMPLE="${1:?usage: headless-run.sh <example-main.tsx> <drive.ts> <MARKER> [tag]}"
DRIVE="${2:?usage: headless-run.sh <example-main.tsx> <drive.ts> <MARKER> [tag]}"
MARKER="${3:?usage: headless-run.sh <example-main.tsx> <drive.ts> <MARKER> [tag]}"
TAG="${4:-$(basename "$(dirname "$EXAMPLE")")}"

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
export WAYLAND_DISPLAY="nd-headless-$TAG"
export GSK_RENDERER=cairo
export GDK_BACKEND=wayland
export NATIVE_AUTOMATION=1
export ND_APP_ID="dev.nativedesktop.headless-$TAG"
# Real fonts, so a capture from this rig looks like a GNOME session instead of
# the bitmap fallback fontconfig picks when it finds no font at all.
. "$(dirname "$0")/headless-fonts.sh"

weston --backend=headless --socket="$WAYLAND_DISPLAY" --idle-time=0 >"$XDG_RUNTIME_DIR/weston-$TAG.log" 2>&1 &
WESTON_PID=$!
# Never `kill "${HOST_PID:-0}"`: the success path clears HOST_PID, and `kill 0`
# signals the whole process group — which on a remote shell takes the session
# down with it.
trap 'kill "$WESTON_PID" 2>/dev/null || true; [ -n "${HOST_PID:-}" ] && kill "$HOST_PID" 2>/dev/null; true' EXIT

for _ in $(seq 1 50); do
  [ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ] && break
  sleep 0.1
done

LOG="$XDG_RUNTIME_DIR/host-$TAG.log"
ND_SCRIPT="$EXAMPLE" ./zig-out/bin/nd-hello >"$LOG" 2>&1 &
HOST_PID=$!

for _ in $(seq 1 600); do
  grep -q "ND_AUTOMATION_LISTENING" "$LOG" && grep -q "ND_COMMIT_APPLIED" "$LOG" && break
  sleep 0.1
done
grep -q "ND_AUTOMATION_LISTENING" "$LOG" || { echo "FAIL: no automation listener"; tail -40 "$LOG"; exit 1; }
grep -q "ND_COMMIT_APPLIED" "$LOG" || { echo "FAIL: no commit applied"; tail -40 "$LOG"; exit 1; }
SOCK=$(grep -m1 "ND_AUTOMATION_LISTENING" "$LOG" | sed 's/.*path=//')

DRIVE_LOG="$XDG_RUNTIME_DIR/drive-$TAG.log"
ND_AUTOMATION_SOCKET="$SOCK" bun "$DRIVE" >"$DRIVE_LOG" 2>&1 \
  || { echo "FAIL: driver"; cat "$DRIVE_LOG"; tail -60 "$LOG"; exit 1; }
cat "$DRIVE_LOG"
grep -q "$MARKER" "$DRIVE_LOG" || { echo "FAIL: driver did not report $MARKER"; exit 1; }

kill -TERM "$HOST_PID"; wait "$HOST_PID" 2>/dev/null || true
HOST_PID=""
echo "headless $TAG: OK ($MARKER)"
