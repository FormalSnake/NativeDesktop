#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
export WAYLAND_DISPLAY=wl-notes-0
export GSK_RENDERER=cairo
export GDK_BACKEND=wayland
export NATIVE_AUTOMATION=1

# Weston's headless output defaults to 1024x640 — too small for the app's
# defaultWidth=1100, which the drive script asserts (window would be clamped).
weston --backend=headless --width=1280 --height=800 --socket="$WAYLAND_DISPLAY" --idle-time=0 &
WESTON_PID=$!
trap 'kill "$WESTON_PID" 2>/dev/null || true' EXIT

for _ in $(seq 1 50); do
  [ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ] && break
  sleep 0.1
done

# scripts/notes-drive.ts now owns launching the host itself (via
# @nativedesktop/test's launchApp): markers, socket parsing, and retries all
# live in the harness, so this wrapper's only job is the Wayland compositor
# and the env vars nd-hello needs once notes-drive.ts spawns it.
ND_SHOT_DIR="$XDG_RUNTIME_DIR" bun scripts/notes-drive.ts gtk >"$XDG_RUNTIME_DIR/drive.log" 2>&1 \
  || { echo "FAIL: driver"; cat "$XDG_RUNTIME_DIR/drive.log"; exit 1; }
cat "$XDG_RUNTIME_DIR/drive.log"
grep -q "ND_NOTES_OK" "$XDG_RUNTIME_DIR/drive.log" || { echo "FAIL: driver did not report success"; exit 1; }
grep -q "ND_NAVCHROME_OK" "$XDG_RUNTIME_DIR/drive.log" || { echo "FAIL: native chrome not present"; exit 1; }
grep -q "ND_THREEPANE_OK" "$XDG_RUNTIME_DIR/drive.log" || { echo "FAIL: three-pane chrome not present"; exit 1; }
grep -q "ND_MENU_NEWNOTE_OK" "$XDG_RUNTIME_DIR/drive.log" || { echo "FAIL: menu bar File>New Note not wired"; exit 1; }

for shot in "$XDG_RUNTIME_DIR/notes-baseline.png" "$XDG_RUNTIME_DIR/notes-final.png"; do
  [ -s "$shot" ] || { echo "FAIL: empty png $shot"; exit 1; }
  file "$shot" | grep -q "PNG image" || { echo "FAIL: not a png $shot"; exit 1; }
done

echo "headless notes: OK (create/edit/search/pin/delete round-trip + screenshots)"
