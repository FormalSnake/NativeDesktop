#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
export WAYLAND_DISPLAY=nd-headless-m9
export GSK_RENDERER=cairo
export GDK_BACKEND=wayland
export ND_APP_VERSION=0.9.0

zig build >/dev/null 2>&1
zig build update-verify >/dev/null 2>&1

# 1. Package the Linux AppImage + signed update artifacts.
bun tools/package.ts linux >"$XDG_RUNTIME_DIR/pkg.log" 2>&1 || { echo "FAIL: package linux"; cat "$XDG_RUNTIME_DIR/pkg.log"; exit 1; }
grep -q ND_PACKAGE_APPIMAGE "$XDG_RUNTIME_DIR/pkg.log" || { echo "FAIL: no AppImage"; cat "$XDG_RUNTIME_DIR/pkg.log"; exit 1; }
PUB=$(grep -m1 ND_PACKAGE_MANIFEST "$XDG_RUNTIME_DIR/pkg.log" | sed 's/.*pub=//')

# 2. Launch the PACKAGED host headlessly (prove the bundle runs). We run the
#    AppDir's binary directly under weston (AppImage FUSE-mount is unavailable in
#    CI sandboxes; the assembled AppDir is the equivalent runnable tree).
weston --backend=headless --socket="$WAYLAND_DISPLAY" --idle-time=0 &
WESTON_PID=$!
trap 'kill "$WESTON_PID" 2>/dev/null || true; kill "${HOST_PID:-0}" 2>/dev/null || true; kill "${SRV_PID:-0}" 2>/dev/null || true' EXIT
for _ in $(seq 1 50); do [ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ] && break; sleep 0.1; done

LOG=$(mktemp)
# examples/counter (not gallery): gallery's <select options={fruits}> passes
# an array-valued prop, which the binary NDP encoder (landed after this
# task's packaging dependency, src/runtime.zig unconditionally advertises
# "binary" in HelloAck) cannot represent (spec §5.3 has no array/object
# tag) — a pre-existing core/runtime gap, not a packaging defect. counter
# has no array/object props and is what the sibling m4/m8 smoke scripts
# already use for this exact kind of packaged-launch proof.
ND_SCRIPT=dist/linux/AppDir/app/examples/counter/main.tsx \
  ./dist/linux/AppDir/usr/bin/nd-hello >"$LOG" 2>&1 &
HOST_PID=$!
for _ in $(seq 1 80); do grep -q ND_COMMIT_APPLIED "$LOG" && break; sleep 0.1; done
grep -q ND_COMMIT_APPLIED "$LOG" || { echo "FAIL: packaged host did not present a commit"; cat "$LOG"; exit 1; }
kill -TERM "$HOST_PID" 2>/dev/null || true; wait "$HOST_PID" 2>/dev/null || true
echo "M9_PACKAGED_LAUNCH_OK"

# 3. Full update flow (verify/download/stage, zero-network — M9-D5).
bun tools/update-server.ts dist/update 0 >"$XDG_RUNTIME_DIR/srv.log" 2>&1 &
SRV_PID=$!
for _ in $(seq 1 50); do grep -q ND_UPDATE_SERVER_LISTENING "$XDG_RUNTIME_DIR/srv.log" && break; sleep 0.1; done
PORT=$(grep -m1 ND_UPDATE_SERVER_LISTENING "$XDG_RUNTIME_DIR/srv.log" | sed 's/.*port=//')
mkdir -p "$XDG_RUNTIME_DIR/m9-stage"
bun scripts/m9-drive.ts "http://127.0.0.1:$PORT" "$PUB" "$XDG_RUNTIME_DIR/m9-stage" >"$XDG_RUNTIME_DIR/drive.log" 2>&1 \
  || { echo "FAIL: update driver"; cat "$XDG_RUNTIME_DIR/drive.log"; exit 1; }
cat "$XDG_RUNTIME_DIR/drive.log"
grep -q M9_UPDATE_OK "$XDG_RUNTIME_DIR/drive.log" || { echo "FAIL: update flow"; exit 1; }
kill "$SRV_PID" 2>/dev/null || true

echo "headless m9: OK (AppImage assembled, packaged host launched, update verify/stage green)"
