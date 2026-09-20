#!/usr/bin/env bash
# Acceptance gate for the window-button layout on the x11 backend. Marker:
# ND_DECORATION_OK.
#
# GTK reads gtk-decoration-layout from XSettings on x11 and from the settings
# portal on Wayland. A compositor with no XSettings manager (wlroots, Hyprland)
# leaves an XWayland client on GTK's compiled-in "menu:close", so the app draws
# a close button on a desktop where every native client draws none. Xvfb with
# no window manager is exactly that session: nothing here publishes XSettings.
#
# Two runs, one per layout, each against a settings portal this script owns, so
# the answer is the gate's and never the capture host's dconf. The assertion is
# what the window PAINTS in the trailing end of its header bar: empty under a
# layout with no buttons, and not empty under one that has close.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"

[ -x "$ROOT/zig-out/bin/nd-hello" ] || { echo "FAIL: build the host first (zig build)"; exit 1; }
[ -f "$ROOT/packages/react/dist/generated/rpc.js" ] || bun run --cwd "$ROOT/packages/react" build >/dev/null

DISPLAY_NUM="${ND_DECO_DISPLAY:-:96}"
WORK="$(mktemp -d)"
STUB="$WORK/portal-settings-stub"
cc "$ROOT/scripts/portal-settings-stub.c" -o "$STUB" $(pkg-config --cflags --libs gio-2.0)

PIDS=()
cleanup() {
  for pid in "${PIDS[@]:-}"; do
    [ -n "$pid" ] || continue
    kill "$pid" 2>/dev/null || true
  done
}
trap cleanup EXIT

Xvfb "$DISPLAY_NUM" -screen 0 1280x800x24 -nolisten tcp >"$WORK/xvfb.log" 2>&1 &
PIDS+=($!)
export DISPLAY="$DISPLAY_NUM"
for _ in $(seq 1 200); do xwininfo -root >/dev/null 2>&1 && break; sleep 0.1; done
xwininfo -root >/dev/null 2>&1 || { echo "FAIL: no X server on $DISPLAY_NUM"; exit 1; }

. "$ROOT/scripts/headless-fonts.sh"
. "$ROOT/scripts/headless-theme.sh"

# One run: a private bus, the stub portal answering `layout`, one host, one
# capture. Echoes "<marker value> <ink pixels>".
run_layout() {
  local layout="$1" tag="$2"
  local dir="$WORK/$tag"
  mkdir -p "$dir/xdg"
  chmod 700 "$dir/xdg"
  cat >"$dir/run.sh" <<INNER
set -euo pipefail
"$STUB" '$layout' >"$dir/stub.log" 2>&1 &
for _ in \$(seq 1 100); do grep -q ND_PORTAL_STUB_READY "$dir/stub.log" && break; sleep 0.1; done
grep -q ND_PORTAL_STUB_READY "$dir/stub.log" || { echo "FAIL: the stub portal never owned the name"; cat "$dir/stub.log"; exit 1; }
export XDG_RUNTIME_DIR="$dir/xdg"
export GDK_BACKEND=x11 GSK_RENDERER=cairo NATIVE_AUTOMATION=1
export ND_SCRIPT=examples/counter/main.tsx
export ND_APP_ID="dev.nativedesktop.headlessDeco$tag\$\$"
"$ROOT/zig-out/bin/nd-hello" >"$dir/host.log" 2>&1 &
HOST=\$!
for _ in \$(seq 1 600); do
  grep -q ND_AUTOMATION_LISTENING "$dir/host.log" && grep -q ND_COMMIT_APPLIED "$dir/host.log" && break
  sleep 0.1
done
grep -q ND_AUTOMATION_LISTENING "$dir/host.log" || { echo "FAIL: the host never came up"; tail -20 "$dir/host.log"; exit 1; }
sleep 1
ND_AUTOMATION_SOCKET="\$(grep -m1 ND_AUTOMATION_LISTENING "$dir/host.log" | sed 's/.*path=//')" \
  ND_DECO_SHOT="$dir/shot.png" bun "$ROOT/scripts/decoration-drive.ts"
kill \$HOST 2>/dev/null || true
INNER
  dbus-run-session -- bash "$dir/run.sh" >"$dir/drive.log" 2>&1 || { echo "FAIL: the $tag run died"; cat "$dir/drive.log"; return 1; }
  cat "$dir/drive.log"
}

echo "== layout ':' (no window buttons) =="
run_layout ':' none
NONE_MARKER="$(grep -m1 ND_DECORATION_LAYOUT "$WORK/none/host.log" || true)"
NONE_INK="$(grep -m1 ND_DECO_INK "$WORK/none/drive.log" | awk '{print $2}')"
echo "  $NONE_MARKER"

echo "== layout ':close' (a close button) =="
run_layout ':close' close
CLOSE_MARKER="$(grep -m1 ND_DECORATION_LAYOUT "$WORK/close/host.log" || true)"
CLOSE_INK="$(grep -m1 ND_DECO_INK "$WORK/close/drive.log" | awk '{print $2}')"
echo "  $CLOSE_MARKER"

FAILED=0
grep -q 'source=portal value=:$' <<<"$NONE_MARKER" || {
  echo "FAIL: the host did not take ':' from the portal (marker: ${NONE_MARKER:-none})"; FAILED=1
}
grep -q 'source=portal value=:close$' <<<"$CLOSE_MARKER" || {
  echo "FAIL: the host did not take ':close' from the portal (marker: ${CLOSE_MARKER:-none})"; FAILED=1
}
# The control run is what makes the empty one mean anything: the same window,
# the same capture, the same corner, with the only difference being the layout.
if [ "${CLOSE_INK:-0}" -lt 100 ]; then
  echo "FAIL: ':close' painted only $CLOSE_INK px in the header's trailing end, so the corner is not where the close button lands"
  FAILED=1
fi
if [ "${NONE_INK:-1}" -ne 0 ]; then
  echo "FAIL: ':' painted $NONE_INK px in the header's trailing end; a layout with no buttons must draw none (':close' drew $CLOSE_INK)"
  FAILED=1
fi

[ "$FAILED" -eq 0 ] || { echo "ND_DECORATION_FAIL captures in $WORK"; exit 1; }
echo "ND_DECORATION_OK x11 follows the portal's button layout: ':' draws $NONE_INK px where ':close' draws $CLOSE_INK"
