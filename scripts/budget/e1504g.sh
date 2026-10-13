#!/usr/bin/env bash
# e1504g budgets (ARCHITECTURE.md 9.5) for an nd example on the GPUI backend, built on g815.
# Usage: e1504g.sh <binary>, from ~/nd-m2-gpui on e1504g holding the binary and the dev
# shell's ld_library_path (docs/plan/baselines.md).
set -u
cd ~/nd-m2-gpui
B=./$1
export XDG_RUNTIME_DIR=/run/user/$(id -u)
export WAYLAND_DISPLAY=$(hyprctl instances -j | jq -r ".[0].wl_socket")
export HYPRLAND_INSTANCE_SIGNATURE=$(hyprctl instances -j | jq -r ".[0].instance")
export LD_LIBRARY_PATH="$(cat ld_library_path)/run/opengl-driver/lib"
export ND_APP_DATA_DIR=$(mktemp -d)
unset VK_ICD_FILENAMES DISPLAY
echo "binary $1 $(stat -c %s $B) bytes"
hyprctl monitors -j | jq -r '.[] | "monitor \(.name) \(.width)x\(.height)@\(.refreshRate) focused=\(.focused)"'
for i in $(seq 10); do
  t0=$(date +%s%3N)
  line=$(ND_EXIT_AFTER_FIRST_FRAME=1 timeout 20 $B 2>/tmp/nd-m2-err.txt | grep first_frame)
  t1=$(echo "$line" | sed -n 's/.*unix_ms=\([0-9]*\).*/\1/p')
  echo "startup run=$i launch_to_first_frame_ms=$((t1 - t0)) $(echo "$line" | sed 's/.*\(since_start_ms=[0-9]*\).*/\1/')"
  sleep 1
done
tail -3 /tmp/nd-m2-err.txt
clk=$(getconf CLK_TCK)
ticks() { awk '{print $14+$15}' /proc/$1/stat; }
$B >/dev/null 2>&1 &
pid=$!
sleep 5
focused=$(hyprctl activewindow -j | jq -r .pid)
a=$(ticks $pid); sleep 60; b=$(ticks $pid)
echo "idle focused=$([ "$focused" = "$pid" ] && echo yes || echo no) cpu_pct=$(awk -v t=$((b-a)) -v c=$clk "BEGIN{printf \"%.3f\", t/c/60*100}") ticks=$((b-a))"
grep -E 'VmRSS|VmHWM' /proc/$pid/status | tr -s ' ' | tr '\n' ' '; echo
other=$(hyprctl clients -j | jq -r --arg p "$pid" '[.[] | select((.pid|tostring) != $p and .mapped)][0].address // empty')
if [ -n "$other" ]; then
  hyprctl dispatch focuswindow "address:$other" >/dev/null
  sleep 2
  a=$(ticks $pid); sleep 60; b=$(ticks $pid)
  echo "idle focused=no cpu_pct=$(awk -v t=$((b-a)) -v c=$clk "BEGIN{printf \"%.3f\", t/c/60*100}") ticks=$((b-a))"
  grep -E 'VmRSS|VmHWM' /proc/$pid/status | tr -s ' ' | tr '\n' ' '; echo
fi
kill $pid; wait $pid 2>/dev/null
for i in 1 2 3; do
  ND_ANIMATE=10 timeout 40 $B 2>/dev/null | grep frames
done
for i in 1 2; do
  ND_BUDGET_CLICKS=40 timeout 60 $B 2>/dev/null | grep input_to_frame
done
rm -rf "$ND_APP_DATA_DIR"
