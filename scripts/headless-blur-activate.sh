#!/usr/bin/env bash
# scripts/blur-activate-drive.ts on GTK under a private Xvfb, since its
# keyboard legs need real key events (xdotool). Run inside `nix develop`.
#
#   scripts/headless-blur-activate.sh [display]      (default :94)
#
# Marker: ND_BLUR_ACTIVATE_OK.
set -euo pipefail
cd "$(dirname "$0")/.."

DISP="${1:-:94}"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
export GSK_RENDERER=cairo
export GDK_BACKEND=x11
export NATIVE_AUTOMATION=1
export ND_APP_ID="dev.nativedesktop.bluractivate${DISP//[^0-9]/}"
export ND_HOST_BINARY="${ND_HOST_BINARY:-$PWD/zig-out/bin/nd-hello}"
. "$(dirname "$0")/headless-fonts.sh"
. "$(dirname "$0")/headless-theme.sh"
if [ -z "${ND_HEADLESS_BUS:-}" ] && command -v dbus-run-session >/dev/null 2>&1; then
  export ND_HEADLESS_BUS=1
  exec dbus-run-session -- "$0" "$DISP"
fi

Xvfb "$DISP" -screen 0 1200x800x24 -nolisten tcp >"$XDG_RUNTIME_DIR/xvfb.log" 2>&1 &
XVFB_PID=$!
trap 'kill "$XVFB_PID" 2>/dev/null || true' EXIT
export DISPLAY="$DISP"
for _ in $(seq 1 50); do
  xdotool getmouselocation >/dev/null 2>&1 && break
  sleep 0.1
done

bun scripts/blur-activate-drive.ts
