#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
export WAYLAND_DISPLAY=nd-headless-m2
export GSK_RENDERER=cairo
export GDK_BACKEND=wayland

weston --backend=headless --socket="$WAYLAND_DISPLAY" --idle-time=0 &
WESTON_PID=$!
trap 'kill "$WESTON_PID" 2>/dev/null || true' EXIT

for _ in $(seq 1 50); do
  [ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ] && break
  sleep 0.1
done

# Run the host for ~4s (enough for handshake + several 500ms uptime commits), then stop it.
OUT=$(timeout 4 ./zig-out/bin/nd-hello 2>&1 || true)
echo "$OUT"

grep -q "ND_HELLO_OK" <<<"$OUT" || { echo "FAIL: no handshake"; exit 1; }
COMMITS=$(grep -c "ND_COMMIT_APPLIED" <<<"$OUT" || true)
[ "$COMMITS" -ge 3 ] || { echo "FAIL: only $COMMITS commits applied"; exit 1; }
echo "headless m2: OK ($COMMITS commits)"
