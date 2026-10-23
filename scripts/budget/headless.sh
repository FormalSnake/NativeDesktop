#!/usr/bin/env bash
# Start-up, idle and frame budgets of one GPUI app in a private headless Hyprland, for hosts whose
# own outputs belong to the user (g815). From the dev shell:
#
#   ndshell headless -- scripts/budget/headless.sh <binary> [args...]
#
# Prints `budget first_frame_ms=..` per launch (launch to the first frame GPUI drew), the median,
# idle CPU and RSS over 20 s with the window focused, and three `ND_ANIMATE=5` frame lines. Extra
# environment for the app (MESSAGES_DEMO=1 and so on) is inherited.
set -u
bin=$1
shift
data=$(mktemp -d)
export ND_APP_DATA_DIR=$data
unset VK_ICD_FILENAMES DISPLAY
runs=()
for i in $(seq 8); do
  t0=$(date +%s%3N)
  line=$(ND_EXIT_AFTER_FIRST_FRAME=1 timeout 30 "$bin" "$@" 2>/dev/null | grep first_frame)
  t1=$(echo "$line" | sed -n 's/.*unix_ms=\([0-9]*\).*/\1/p')
  [ -n "$t1" ] || { echo "budget run=$i no first frame"; continue; }
  runs+=($((t1 - t0)))
  echo "budget run=$i first_frame_ms=$((t1 - t0))"
done
# The first launch pays for cold caches; the median is over the rest.
[ ${#runs[@]} -gt 1 ] && echo "budget first_frame_median_ms=$(printf '%s\n' "${runs[@]:1}" | sort -n | awk '{a[NR]=$1} END {print a[int((NR+1)/2)]}')"
clk=$(getconf CLK_TCK)
ticks() { awk '{print $14+$15}' "/proc/$1/stat"; }
"$bin" "$@" >/dev/null 2>&1 &
pid=$!
sleep 5
a=$(ticks $pid)
sleep 20
b=$(ticks $pid)
echo "budget idle cpu_pct=$(awk -v t=$((b - a)) -v c="$clk" 'BEGIN{printf "%.2f", t/c/20*100}') $(grep -E 'VmRSS|VmHWM' /proc/$pid/status | tr -s ' \t' ' ' | tr '\n' ' ')"
kill $pid
wait $pid 2>/dev/null
for i in 1 2 3; do
  ND_ANIMATE=5 timeout 40 "$bin" "$@" 2>/dev/null | grep 'nd-budget: frames' | sed 's/^nd-budget:/budget/'
done
rm -rf -- "${data:?}"
