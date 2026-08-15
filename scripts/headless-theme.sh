#!/usr/bin/env bash
# Sourced by the headless gates (right after headless-fonts.sh). Pins the GTK
# THEMING a capture is judged against, so a screenshot taken on a developer's
# box renders stock Adwaita rather than that box's desktop.
#
# Two channels leak a live session into a "headless" run, and both had to be
# closed before the Stage-5 HIG pass could trust a colour reading:
#   1. `$XDG_CONFIG_HOME/gtk-4.0/gtk.css` — user CSS redefining
#      `@define-color window_bg_color` / `headerbar_bg_color` / … . On this
#      project's own capture host that turned every chrome surface into a
#      third-party palette (`#101418` header bar, `#1d2024` sidebar) and the
#      review read it as the framework hardcoding colours.
#   2. the session settings portal over the user DBus — font, icon theme and
#      colour scheme, which beats both `settings.ini` and GSettings.
# A generated XDG_CONFIG_HOME closes (1). Closing (2) needs a private bus,
# which a sourced file cannot enter: the CALLER re-execs itself under
# `dbus-run-session` (see scripts/headless-run.sh).
#
# Not executable on its own — `. scripts/headless-theme.sh`.
#
# Colour scheme is the caller's choice: export ADW_DEBUG_COLOR_SCHEME
# (`prefer-dark` / `prefer-light`) before sourcing to pin one, otherwise
# libadwaita's own default stands.

nd_theme_dir="${XDG_RUNTIME_DIR:-/tmp}/nd-headless-theme"
mkdir -p "$nd_theme_dir/gtk-4.0" "$nd_theme_dir/glib-2.0/settings"
# GNOME 48 replaced Cantarell and Source Code Pro as the system fonts; the
# families here have to match the ones headless-fonts.sh makes resolvable.
#
# gtk-enable-animations=0 is a capture requirement, not a preference. A
# surface that presents with a fade is captured on whichever frame the
# snapshot happened to catch: the Stage-5 palette shot came out mid-present at
# roughly 20% opacity (card fill rgb(249) over a scrimmed page of rgb(248)),
# and every colour read off it was wrong. With animations off, a presented
# surface is at its final opacity on the frame it first appears.
printf '[Settings]\ngtk-font-name = Adwaita Sans 11\ngtk-icon-theme-name = Adwaita\ngtk-enable-animations = 0\n' \
  >"$nd_theme_dir/gtk-4.0/settings.ini"
printf "[org/gnome/desktop/interface]\nfont-name='Adwaita Sans 11'\ndocument-font-name='Adwaita Sans 11'\nmonospace-font-name='Adwaita Mono 11'\nicon-theme-name='Adwaita'\nenable-animations=false\n" \
  >"$nd_theme_dir/glib-2.0/settings/keyfile"
export XDG_CONFIG_HOME="$nd_theme_dir"
export GSETTINGS_BACKEND=keyfile
