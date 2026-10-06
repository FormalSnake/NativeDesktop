#!/usr/bin/env bash
# Nested Hyprland (headless mutter parent) for perf runs; runs "$2" with the
# rig's DISPLAY/WAYLAND_DISPLAY exported, inside the framework's dev shell (it
# reads ND_PIN_*). Usage: rig.sh <run-name> <inner.sh> [args]; output lands in
# $SNAPPY_RUNS/<run-name> (default ~/snappy-runs).
set -uo pipefail
RUN="${SNAPPY_RUNS:-$HOME/snappy-runs}/$1"; shift
INNER="$1"; shift
mkdir -p "$RUN"
PIDS=()
kill_tree() { local c; for c in $(pgrep -P "$1" 2>/dev/null); do kill_tree "$c"; done; kill "$1" 2>/dev/null || true; }
RT="$(mktemp -d /tmp/ndsnap.XXXXXX)"; chmod 700 "$RT"
cleanup() { for p in "${PIDS[@]:-}"; do [ -n "$p" ] && kill_tree "$p"; done; sleep 1; rm -rf "$RT"; }
trap cleanup EXIT
pin_gl=(env __EGL_VENDOR_LIBRARY_FILENAMES="$ND_PIN_EGL_VENDOR_FILE" LIBGL_DRIVERS_PATH="$ND_PIN_DRI_PATH" GBM_BACKENDS_PATH="$ND_PIN_GBM_PATH")
( export XDG_RUNTIME_DIR="$RT"; unset DISPLAY WAYLAND_DISPLAY
  exec "${pin_gl[@]}" dbus-run-session -- mutter --headless --virtual-monitor 1920x1080 --wayland-display nd-parent --no-x11 ) >"$RUN/mutter.log" 2>&1 &
PIDS+=($!)
for _ in $(seq 1 400); do [ -S "$RT/nd-parent" ] && break; sleep 0.1; done
printf '%s\n' 'xwayland {' '  enabled = true' '  force_zero_scaling = true' '}' 'misc {' '  disable_hyprland_logo = true' '  disable_splash_rendering = true' '  force_default_wallpaper = 0' '}' 'animations {' '  enabled = false' '}' 'general {' '  border_size = 0' '  gaps_in = 0' '  gaps_out = 0' '}' >"$RUN/hypr.conf"
( export XDG_RUNTIME_DIR="$RT" WAYLAND_DISPLAY=nd-parent HYPRLAND_NO_CRASHREPORTER=1 HYPRLAND_NO_RT=1 LIBSEAT_BACKEND=logind; unset DISPLAY
  exec "${pin_gl[@]}" Hyprland -c "$RUN/hypr.conf" ) >"$RUN/hypr.log" 2>&1 &
HYPR=$!; PIDS+=($HYPR)
export XDG_RUNTIME_DIR="$RT"
for _ in $(seq 1 600); do HYPRLAND_INSTANCE_SIGNATURE="$(ls -t "$RT/hypr" 2>/dev/null | head -1)"; [ -n "$HYPRLAND_INSTANCE_SIGNATURE" ] && [ -S "$RT/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock" ] && break; sleep 0.1; done
export HYPRLAND_INSTANCE_SIGNATURE
for _ in $(seq 1 300); do [ "$(hyprctl -j monitors 2>/dev/null)" = "[]" ] && break; sleep 0.1; done
hyprctl output create headless >/dev/null; sleep 1
MON="$(hyprctl -j monitors | grep -o '"name": *"[^"]*"' | head -1 | sed 's/.*"\([^"]*\)"$/\1/')"
hyprctl keyword monitor "$MON,1920x1080@60,0x0,1" >/dev/null; sleep 1
for _ in $(seq 1 600); do XD="$(ps -o args= --ppid "$HYPR" 2>/dev/null | sed -n 's/^Xwayland \(:[0-9][0-9]*\).*/\1/p' | head -1)"; [ -n "$XD" ] && break; sleep 0.1; done
export DISPLAY="$XD"
export WAYLAND_DISPLAY="$(ls -t "$RT" | grep -x 'wayland-[0-9]*' | head -1)"
for _ in $(seq 1 200); do xwininfo -root >/dev/null 2>&1 && break; sleep 0.1; done
echo "rig up: DISPLAY=$DISPLAY WAYLAND_DISPLAY=$WAYLAND_DISPLAY monitor=$MON rt=$RT"
export RUN
bash "$INNER" "$@"
echo "rig done rc=$?"
