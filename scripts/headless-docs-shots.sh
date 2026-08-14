#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
# scripts/headless-docs-shots.sh — runs scripts/docs-shots.ts against the GTK
# host under a headless weston (same recipe as the other headless-*.sh
# wrappers), pinned to the light color scheme for the docs-site screenshots.
#   ./scripts/headless-docs-shots.sh <outDir> [example ...]
OUT="${1:?usage: headless-docs-shots.sh <outDir> [example ...]}"
shift || true

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
export WAYLAND_DISPLAY=wl-docs-shots-0
# cairo is the known-good headless renderer, and it does rasterize live
# <webview> content: browser and multiwindow come out pixel-true here.
# ND_SHOT_RENDERER=gl swaps in llvmpipe software GL if a shot ever needs it.
#
# An earlier note in this spot blamed the private bus below for the flat
# "WebView" placeholder plate (ND_SNAPSHOT_DEGRADED on host stderr) and sent
# the reader off to a session-bus host. That was wrong on both counts. The
# page loads under the private bus, and the plate came from vtSnapshot taking
# its WidgetPaintable pass while the window still owed a layout pass, which
# frees to a null render node whatever is in the window. src/gtk/backend.zig
# now retries that pass behind a frame-clock pump; webview windows here have
# needed up to five ticks. WEBKIT_DISABLE_DMABUF_RENDERER and
# WEBKIT_DISABLE_COMPOSITING_MODE only shifted the odds and are not needed.
export GSK_RENDERER="${ND_SHOT_RENDERER:-cairo}"
export GDK_BACKEND=wayland
export ADW_DEBUG_COLOR_SCHEME=prefer-light
export ND_BACKEND=gtk

# Pin the UI font. GTK's GSettings backend wins over gtk-4.0/settings.ini,
# and its default (Cantarell) is not installed here, so fontconfig would
# substitute the machine's themed "sans" (a bitmap font on this box). Force
# the keyfile GSettings backend with an explicit font-name, and alias the
# generic families to Noto Sans for anything that asks fontconfig directly.
FONT_CONF_DIR="$(mktemp -d)"
mkdir -p "$FONT_CONF_DIR/gtk-4.0" "$FONT_CONF_DIR/glib-2.0/settings" "$FONT_CONF_DIR/fontconfig"
printf '[Settings]\ngtk-font-name = Noto Sans 10\ngtk-icon-theme-name = Adwaita\n' > "$FONT_CONF_DIR/gtk-4.0/settings.ini"
printf "[org/gnome/desktop/interface]\nfont-name='Noto Sans 10'\ndocument-font-name='Noto Sans 10'\nmonospace-font-name='Noto Sans Mono 10'\nicon-theme-name='Adwaita'\n" \
  > "$FONT_CONF_DIR/glib-2.0/settings/keyfile"
cat > "$FONT_CONF_DIR/fontconfig/fonts.conf" <<'FCEOF'
<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "fonts.dtd">
<fontconfig>
  <include ignore_missing="yes">/etc/fonts/fonts.conf</include>
  <match target="pattern"><test name="family"><string>sans</string></test><edit name="family" mode="prepend" binding="strong"><string>Noto Sans</string></edit></match>
  <match target="pattern"><test name="family"><string>sans-serif</string></test><edit name="family" mode="prepend" binding="strong"><string>Noto Sans</string></edit></match>
  <match target="pattern"><test name="family"><string>Cantarell</string></test><edit name="family" mode="prepend" binding="strong"><string>Noto Sans</string></edit></match>
  <match target="pattern"><test name="family"><string>monospace</string></test><edit name="family" mode="prepend" binding="strong"><string>Noto Sans Mono</string></edit></match>
</fontconfig>
FCEOF
export XDG_CONFIG_HOME="$FONT_CONF_DIR"
export FONTCONFIG_FILE="$FONT_CONF_DIR/fontconfig/fonts.conf"
export GSETTINGS_BACKEND=keyfile
# Without this, GDK asks the LIVE desktop session's settings portal over the
# user DBus for fonts/appearance/icon theme, and the machine's theming leaks
# into the "headless" run, overriding both settings.ini and the keyfile above.
# UNSETTING the variable is not enough: GDBus then falls back to
# $XDG_RUNTIME_DIR/bus, which on a logged-in capture host IS the session bus
# (observed: gtk-icon-theme-name came back Colloid-Dark, gtk-font-name MEK
# Sans 11). Pointing it at a nonexistent socket does defeat the portal, but it
# also breaks WebKitGTK: its network process launches a bwrap'd dbus-proxy
# against this address and aborts the host when the path does not resolve, so
# the browser and multiwindow captures die. A PRIVATE session bus satisfies
# both: it is a real bus WebKit can proxy, and it carries no settings portal,
# so the keyfile backend still wins. `exec` below re-enters this script under
# dbus-run-session once.
if [ -z "${ND_DOCS_SHOTS_BUS:-}" ]; then
  export ND_DOCS_SHOTS_BUS=1
  exec dbus-run-session -- "$0" "$OUT" "$@"
fi

# The <terminal> example spawns $SHELL; a plain sh with a bare prompt beats a
# host rc file's escape-laden PS1 in a documentation screenshot.
export SHELL=/bin/sh
export PS1='$ '

# 1600x1000 clears the widest example (notes, defaultWidth=1100) with margin.
weston --backend=headless --width=1600 --height=1000 --socket="$WAYLAND_DISPLAY" --idle-time=0 &
WESTON_PID=$!
trap 'kill "$WESTON_PID" 2>/dev/null || true' EXIT

for _ in $(seq 1 50); do
  [ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ] && break
  sleep 0.1
done
[ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ] || { echo "FAIL: weston socket never appeared"; exit 1; }

echo "docs-shots font: XDG_CONFIG_HOME=$XDG_CONFIG_HOME sans=$(fc-match sans 2>/dev/null)" >&2

# Fail fast: wrong icons used to ship silently. Neither half of this is
# answerable from the shell — there is no gtk4-query-settings binary, and a
# .svg on XDG_DATA_DIRS says nothing about which theme GTK picked or whether
# the glyph has any ink. Ask GTK, on the live display, once the compositor is
# up. Adwaita itself reaches GTK through the XDG_DATA_DIRS `nix develop`
# builds from flake.nix's devShell packages.
ICON_PROBE="$(mktemp -d)/gtk-icon-probe"
cc scripts/gtk-icon-probe.c -o "$ICON_PROBE" $(pkg-config --cflags --libs gtk4)
"$ICON_PROBE" Adwaita pan-down-symbolic system-search-symbolic network-transmit-receive-symbolic >&2

bun scripts/docs-shots.ts gtk "$OUT" "$@"
