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
# cairo is the known-good headless renderer, but its snapshot path cannot
# rasterize a <webview>'s texture (screenshot answers -32603); the webview
# examples re-run with ND_SHOT_RENDERER=gl (llvmpipe software GL).
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
printf '[Settings]\ngtk-font-name = Noto Sans 10\n' > "$FONT_CONF_DIR/gtk-4.0/settings.ini"
printf "[org/gnome/desktop/interface]\nfont-name='Noto Sans 10'\ndocument-font-name='Noto Sans 10'\nmonospace-font-name='Noto Sans Mono 10'\n" \
  > "$FONT_CONF_DIR/glib-2.0/settings/keyfile"
cat > "$FONT_CONF_DIR/fontconfig/fonts.conf" <<'FCEOF'
<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "fonts.dtd">
<fontconfig>
  <include ignore_missing="yes">/etc/fonts/fonts.conf</include>
  <match target="pattern"><test name="family"><string>sans</string></test><edit name="family" mode="prepend" binding="strong"><string>Noto Sans</string></edit></match>
  <match target="pattern"><test name="family"><string>sans-serif</string></test><edit name="family" mode="prepend" binding="strong"><string>Noto Sans</string></edit></match>
  <match target="pattern"><test name="family"><string>Cantarell</string></test><edit name="family" mode="prepend" binding="strong"><string>Noto Sans</string></edit></match>
  <match target="pattern"><test name="family"><string>MEK Sans</string></test><edit name="family" mode="assign" binding="strong"><string>Noto Sans</string></edit></match>
  <match target="pattern"><test name="family"><string>monospace</string></test><edit name="family" mode="prepend" binding="strong"><string>Noto Sans Mono</string></edit></match>
</fontconfig>
FCEOF
export XDG_CONFIG_HOME="$FONT_CONF_DIR"
export FONTCONFIG_FILE="$FONT_CONF_DIR/fontconfig/fonts.conf"
export GSETTINGS_BACKEND=keyfile
# Without this, GDK asks the LIVE desktop session's settings portal over the
# user DBus for fonts/appearance, and the machine's theming (bitmap fonts,
# dark scheme) leaks into the "headless" run. No bus -> keyfile backend wins.
unset DBUS_SESSION_BUS_ADDRESS

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

bun scripts/docs-shots.ts gtk "$OUT" "$@"
