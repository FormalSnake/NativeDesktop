#!/usr/bin/env bash
# M1 gate for the Chromium <webview> engine: runs examples/cef-probe with
# ND_WEBVIEW_ENGINE=chromium and drives it with scripts/cef-drive.ts.
# Marker: ND_CEF_M1_OK.
#
# Xvfb, not weston. CEF's windowed embedding is compiled X11-only, so the view
# lives in an X11 child window of the host toplevel; under weston --xwayland
# that runs, but XWayland's root window paints nothing, so there is no capture
# that shows the page and no toplevel census that means anything. A plain X
# server composites every window into a real root, which gives this gate both
# its screenshot and its no-stray-window proof.
set -euo pipefail
cd "$(dirname "$0")/.."

: "${ND_CEF_DIST:=$HOME/.cache/nativedesktop/cef/151.3.23-linux64}"
[ -f "$ND_CEF_DIST/Release/libcef.so" ] || { echo "SKIP: no CEF distribution at $ND_CEF_DIST"; exit 0; }
# Chromium resolves icudtl.dat and the .pak files against libcef.so's own
# directory, and it does so inside cef_initialize before `resources_dir_path`
# is applied, so an unpacked distribution's Release/Resources split aborts the
# process. `nd package` stages them together; for the dev cache, do the same
# here with symlinks. Idempotent.
ln -sfn "$ND_CEF_DIST"/Resources/* "$ND_CEF_DIST/Release/"
export ND_CEF_ROOT="$ND_CEF_DIST/Release"

# libcef.so is a generic-Linux binary; the flake devshell exports the closure
# of the 26 sonames it needs (see flake.nix cefRuntimeLibs).
if [ -n "${ND_CEF_LD_LIBRARY_PATH:-}" ]; then
  export LD_LIBRARY_PATH="$ND_CEF_LD_LIBRARY_PATH${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
export DISPLAY="${ND_CEF_DISPLAY:-:97}"
export GDK_BACKEND=x11
export GSK_RENDERER=cairo
export NATIVE_AUTOMATION=1
export ND_WEBVIEW_ENGINE=chromium
export ND_APP_ID="${ND_APP_ID:-dev.nativedesktop.headlessCef}"
XDG_DATA_HOME="$(mktemp -d)"
export XDG_DATA_HOME

Xvfb "$DISPLAY" -screen 0 1280x900x24 -nolisten tcp >/dev/null 2>&1 &
XVFB_PID=$!
# Never `kill "${HOST_PID:-0}"`: the success path clears HOST_PID, and `kill 0`
# signals the whole process group, which on a remote shell takes the session
# down with it.
trap 'kill "$XVFB_PID" 2>/dev/null || true; [ -n "${HOST_PID:-}" ] && kill "$HOST_PID" 2>/dev/null; true' EXIT

for _ in $(seq 1 100); do
  xwininfo -root >/dev/null 2>&1 && break
  sleep 0.1
done
xwininfo -root >/dev/null 2>&1 || { echo "FAIL: Xvfb never came up on $DISPLAY"; exit 1; }

# The census counts windows a user could see: mapped, and at least 200x200.
# Chromium keeps a handful of 1x1 and 10x10 utility windows on the root
# (clipboard owner, drag proxy) that are never presented, and counting those
# would make the invariant unfalsifiable rather than strict.
toplevels() {
  xwininfo -root -children |
    sed -n 's/^ *\(0x[0-9a-f]*\).*[^0-9]\([0-9]\+\)x\([0-9]\+\)+.*/\1 \2 \3/p' |
    while read -r id w h; do
      [ "$w" -ge 200 ] && [ "$h" -ge 200 ] || continue
      # A window can vanish between the listing and the query, so neither the
      # miss nor the non-match may take `set -e` with it.
      if xwininfo -id "$id" 2>/dev/null | grep -q "Map State: IsViewable"; then echo "$id"; fi
    done
  true
}
BEFORE_X11="$(toplevels | wc -l)"

LOG=$(mktemp)
# The trace names the parent, container and CEF window XIDs plus the allocation
# they were created against; an embedding failure is invisible without them.
ND_WEBVIEW_TRACE=1 ND_SCRIPT=examples/cef-probe/main.tsx ./zig-out/bin/nd-hello >"$LOG" 2>&1 &
HOST_PID=$!

for _ in $(seq 1 900); do
  grep -q "ND_AUTOMATION_LISTENING" "$LOG" && grep -q "ND_COMMIT_APPLIED" "$LOG" && break
  sleep 0.1
done
grep -q "ND_AUTOMATION_LISTENING" "$LOG" || { echo "FAIL: no automation listener"; cat "$LOG"; exit 1; }
grep -q "ND_COMMIT_APPLIED" "$LOG" || { echo "FAIL: no commit applied"; cat "$LOG"; exit 1; }

# The engine resolves lazily, at the app's first <webview>.
for _ in $(seq 1 600); do
  grep -q "ND_WEBVIEW_ENGINE chromium" "$LOG" && break
  sleep 0.1
done
grep -q "ND_WEBVIEW_ENGINE chromium" "$LOG" || {
  echo "FAIL: the chromium engine did not load"
  tail -40 "$LOG"
  exit 1
}
# A fallback to WebKitGTK would still pass every event check below, so the
# absence of the fallback warning is part of the gate.
grep -q "falling back to the system engine" "$LOG" && {
  echo "FAIL: the view fell back to the system engine"
  grep -n "ND_WARN" "$LOG" | head -20
  exit 1
}

SOCK=$(grep -m1 "ND_AUTOMATION_LISTENING" "$LOG" | sed 's/.*path=//')
ND_AUTOMATION_SOCKET="$SOCK" ND_SHOT_PATH="$XDG_RUNTIME_DIR/cef-probe-host.png" \
  bun scripts/cef-drive.ts >"$XDG_RUNTIME_DIR/drive.log" 2>&1 \
  || { echo "FAIL: driver"; cat "$XDG_RUNTIME_DIR/drive.log"; tail -80 "$LOG"; exit 1; }
cat "$XDG_RUNTIME_DIR/drive.log"
grep -q "ND_CEF_M1_OK" "$XDG_RUNTIME_DIR/drive.log" || { echo "FAIL: driver did not report success"; exit 1; }

# The no-stray-window invariant, measured at the X server rather than taken on
# trust. The drive has already run the popup leg by this point, so a
# CEF-created top-level would be on the root's child list now. The host's own
# window is the one legitimate addition.
AFTER_X11="$(toplevels | wc -l)"
ADDED=$((AFTER_X11 - BEFORE_X11))
if [ "$ADDED" -gt 1 ]; then
  echo "FAIL: $ADDED new X11 toplevels after the popup leg, want at most 1 (the host window)"
  xwininfo -root -children
  exit 1
fi
echo "ND_CEF_NO_STRAY_WINDOW_OK toplevels $BEFORE_X11 -> $AFTER_X11 across the popup leg"

# The picture that proves a page rendered inside the host window: the X root,
# which composites the CEF child window the host's own snapshot cannot reach.
SHOT="${ND_CEF_SHOT_PATH:-$XDG_RUNTIME_DIR/cef-probe-x11.png}"
import -window root "$SHOT"
[ -s "$SHOT" ] || { echo "FAIL: empty X11 capture"; exit 1; }
file "$SHOT" | grep -q "PNG image" || { echo "FAIL: not a png"; exit 1; }

# A capture of the right size proves nothing on its own: an embedding that
# never painted leaves the host's own light background there. The probe's
# fixture is deliberately dark (#101014), so the mean brightness of a rectangle
# well inside the view separates "Chromium painted" from "GTK did".
PAGE_MEAN=$(magick "$SHOT" -crop 700x260+120+380 +repage -format "%[fx:mean]" info: 2>/dev/null || echo "")
[ -n "$PAGE_MEAN" ] || { echo "FAIL: could not measure the captured view"; exit 1; }
awk -v m="$PAGE_MEAN" 'BEGIN { exit !(m < 0.5) }' || {
  echo "FAIL: the embedded view is not showing the page (mean brightness $PAGE_MEAN, want the fixture's dark background)"
  exit 1
}
echo "ND_CEF_PAGE_PAINTED_OK mean brightness $PAGE_MEAN inside the embedded view"

kill -TERM "$HOST_PID"; wait "$HOST_PID" 2>/dev/null || true
HOST_PID=""

echo "headless webview (chromium): OK (M1 events verified, X11 capture at $SHOT)"
