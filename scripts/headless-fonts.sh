#!/usr/bin/env bash
# Sourced by the headless gates. The flake dev shell already exports
# FONTCONFIG_FILE (see flake.nix), which is the path that matters; this only
# covers a gate run OUTSIDE the dev shell, where inheriting the host's
# /etc/fonts leaves `sans` unresolvable and GTK renders a bitmap fallback.
#
# Not executable on its own — `. scripts/headless-fonts.sh`.

if [ -n "${FONTCONFIG_FILE:-}" ] && [ -r "${FONTCONFIG_FILE}" ]; then
  return 0 2>/dev/null || exit 0
fi

nd_font_dirs=()
for d in \
  "$HOME/.nix-profile/share/fonts" \
  /run/current-system/sw/share/fonts \
  /usr/share/fonts \
  /usr/local/share/fonts \
  "$HOME/.local/share/fonts"; do
  [ -d "$d" ] && nd_font_dirs+=("$d")
done

if [ ${#nd_font_dirs[@]} -eq 0 ]; then
  echo "ND_WARN headless-fonts: no font directory found; text will render in the bitmap fallback" >&2
  return 0 2>/dev/null || exit 0
fi

nd_conf="${XDG_RUNTIME_DIR:-/tmp}/nd-headless-fonts.conf"
{
  echo '<?xml version="1.0"?>'
  echo '<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">'
  echo '<fontconfig>'
  for d in "${nd_font_dirs[@]}"; do echo "  <dir>$d</dir>"; done
  echo "  <cachedir>${XDG_RUNTIME_DIR:-/tmp}/nd-fontconfig-cache</cachedir>"
  # GNOME's UI font first, then a face with broad coverage. Without an explicit
  # alias, fontconfig with no system config answers whatever sorts first.
  for family in sans-serif sans; do
    echo "  <alias><family>$family</family><prefer>"
    echo '    <family>Cantarell</family><family>DejaVu Sans</family>'
    echo '  </prefer></alias>'
  done
  echo '  <alias><family>monospace</family><prefer><family>DejaVu Sans Mono</family></prefer></alias>'
  echo '  <match target="font"><edit name="antialias" mode="assign"><bool>true</bool></edit></match>'
  echo '  <match target="font"><edit name="hinting" mode="assign"><bool>true</bool></edit></match>'
  echo '</fontconfig>'
} >"$nd_conf"
export FONTCONFIG_FILE="$nd_conf"
