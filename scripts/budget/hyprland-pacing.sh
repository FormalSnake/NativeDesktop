#!/usr/bin/env bash
# hello and the two-window pacing example in g815's live Hyprland session (144 Hz HDMI-A-1,
# 240 Hz eDP-1). Usage: hyprland-pacing.sh <suffix> [x11], from ~/nd-build holding hello-<suffix>,
# pacing-<suffix> and ld_library_path. `x11` runs them under XWayland.
set -u
cd ~/nd-build
SUF=$1
export XDG_RUNTIME_DIR=/run/user/$(id -u)
export HYPRLAND_INSTANCE_SIGNATURE=$(hyprctl instances -j | jq -r ".[0].instance")
export LD_LIBRARY_PATH="$(cat ld_library_path)/run/opengl-driver/lib"
export ND_APP_DATA_DIR=$(mktemp -d)
unset VK_ICD_FILENAMES
if [ "${2:-}" = x11 ]; then
  unset WAYLAND_DISPLAY
  export DISPLAY=$(ls /tmp/.X11-unix | sed 's/X/:/' | head -1)
  echo "x11 DISPLAY=$DISPLAY"
else
  export WAYLAND_DISPLAY=$(hyprctl instances -j | jq -r ".[0].wl_socket")
  unset DISPLAY
fi
hyprctl monitors -j | jq -r '.[] | "monitor \(.name) \(.width)x\(.height)@\(.refreshRate) focused=\(.focused)"'
hyprctl dispatch focusmonitor HDMI-A-1 >/dev/null
./hello-$SUF > hello.out 2>hello.err &
pid=$!
sleep 3
hyprctl clients -j | jq -r --arg p "$pid" '.[] | select((.pid|tostring) == $p) | "hello window \(.title) at \(.at) size \(.size) monitor=\(.monitor) xwayland=\(.xwayland)"'
geom=$(hyprctl clients -j | jq -r --arg p "$pid" '.[] | select((.pid|tostring) == $p) | "\(.at[0]),\(.at[1]) \(.size[0])x\(.size[1])"' | head -1)
[ -n "$geom" ] && grim -g "$geom" hello-$SUF${2:+-$2}.png && echo "shot hello-$SUF${2:+-$2}.png"
kill $pid; wait $pid 2>/dev/null
ND_PACING=1 ./pacing-$SUF > /dev/null 2> pacing.err &
pid=$!
sleep 4
echo "both on HDMI-A-1 (144 Hz):"
grep nd-pacing pacing.err | tail -2
addr=$(hyprctl clients -j | jq -r --arg p "$pid" '.[] | select((.pid|tostring) == $p and .title == "pacing-b") | .address')
ws=$(hyprctl monitors -j | jq -r '.[] | select(.name == "eDP-1") | .activeWorkspace.id')
hyprctl dispatch "hl.dsp.window.move({ window = \"address:$addr\", workspace = $ws, follow = false })"
sleep 5
hyprctl clients -j | jq -r --arg p "$pid" '.[] | select((.pid|tostring) == $p) | "\(.title) monitor=\(.monitor)"'
echo "after moving pacing-b to eDP-1 (240 Hz):"
grep nd-pacing pacing.err | tail -2
kill $pid; wait $pid 2>/dev/null
rm -rf "$ND_APP_DATA_DIR"
