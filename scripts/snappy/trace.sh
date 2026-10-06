#!/usr/bin/env bash
# Where the UI thread's time goes during the interactions that hitch on a slow
# laptop, inside rig.sh or xrig.sh: typing in the command bar, wheel over a
# long sidebar, and typing while ten tabs reload in a loop. The host runs with
# ND_PERF_TRACE=1; each phase is bracketed by `ND_PHASE <name> <begin|end>
# at=<wall us>` so summarize.ts can bucket the trace lines, and the outside
# latency of every keystroke and wheel notch lands in lat.txt.
# Usage: trace.sh <name> <fw-dir> <app-dir>
set -u
NAME=$1 FW=$2 APPD=$3
HERE=$(cd "$(dirname "$0")" && pwd)
LAT=$HERE/../latency-x11.ts
OUT=$RUN/$NAME; mkdir -p $OUT
kill_tree() { local c; for c in $(pgrep -P "$1" 2>/dev/null); do kill_tree "$c"; done; kill "$1" 2>/dev/null || true; }
phase() { echo "ND_PHASE $1 $2 at=$(date +%s%6N)" >>$OUT/host.log; }
PORT=$((9900 + RANDOM % 50))
bun $HERE/fixture.ts $PORT >$OUT/fixture.log 2>&1 & FX=$!
sleep 1
F=http://127.0.0.1:$PORT/

rm -rf $OUT/p; mkdir -p $OUT/p/store $OUT/p/data $OUT/p/cache
TABS=""
for i in $(seq 1 60); do TABS="$TABS${TABS:+,}{\"id\":\"t$i\",\"url\":\"$F?tab=$i\",\"title\":\"Fixture $i\",\"pinned\":false}"; done
echo '{"version":1,"data":{"tabs":['$TABS'],"activeId":"t1","nextTabId":61,"windowWidth":1920,"windowHeight":1080,"zoomByHost":{}}}' >$OUT/p/store/session.json
echo '{"version":1,"data":{"searchEngine":"duckduckgo","homepage":"","restoreOnLaunch":true,"layout":"sidebar","pinnedExtensions":[]}}' >$OUT/p/store/settings.json

( cd $APPD
  exec env LD_LIBRARY_PATH=$(dirname $ND_PIN_DRI_PATH):$ND_CEF_LD_LIBRARY_PATH:$LD_LIBRARY_PATH \
    __EGL_VENDOR_LIBRARY_FILENAMES=$ND_PIN_EGL_VENDOR_FILE LIBGL_DRIVERS_PATH=$ND_PIN_DRI_PATH GBM_BACKENDS_PATH=$ND_PIN_GBM_PATH \
    ND_CEF_ROOT=$HOME/.cache/nativedesktop/cef/151.3.23-linux64/Release ND_PERF_TRACE=1 \
    XDG_DATA_HOME=$OUT/p/data ND_CEF_CACHE=$OUT/p/cache NB_STORE_DIR=$OUT/p/store \
    GDK_BACKEND=x11 NATIVE_AUTOMATION=1 ND_WEBVIEW_ENGINE=chromium ND_CEF_STYLE=chrome ND_SCRIPT=src/main.tsx \
    ND_APP_ID=dev.snappy.$NAME ${NB_ENV:-} setsid $FW/zig-out/bin/nd-hello --remote-debugging-port=$((PORT + 1)) --remote-allow-origins='*' ) >>$OUT/host.log 2>&1 &
BPID=$!
lat() { timeout 300 bun $LAT $BPID "$@" 2>&1 | tee -a $OUT/lat.txt | grep -c ND_LAT; }

lat startup:0,0,400,300 until:page_painted:1300,700:2a6fdb >/dev/null
sleep 8

# Ten tabs get a browser each (a restored tab only loads once it is shown).
phase wake begin
steps=(); for i in $(seq 1 9); do steps+=(key:Control_L+Tab wait:700); done
lat "${steps[@]}" >/dev/null
phase wake end
sleep 5

phase idle begin; sleep 5; phase idle end

BAR=560,60,800,420
type_word() { local s=(); for k in g i t h u b; do s+=(measure:$1_$k key:$k wait:${2:-250}); done; echo "${s[@]}"; }
phase type begin
steps=(); for _ in 1 2 3; do steps+=(region:$BAR measure:open key:Control_L+l wait:600 $(type_word type) key:Escape wait:600); done
lat "${steps[@]}" >/dev/null
phase type end
sleep 2

# The sidebar holds 60 rows, more than fit, so each notch scrolls it.
phase wheel begin
steps=(region:0,80,300,900 move:150,500 wait:300)
for _ in $(seq 1 12); do steps+=(measure:wheel_down wheel:150,500,down wait:120); done
for _ in $(seq 1 12); do steps+=(measure:wheel_up wheel:150,500,up wait:120); done
lat "${steps[@]}" >/dev/null
phase wheel end
sleep 2

phase load begin
HOST=$(pgrep -s $BPID -f zig-out/bin/nd-hello | while read p; do tr "\0" " " </proc/$p/cmdline | grep -q -- --type= || echo $p; done | head -1)
[ -n "${PERF:-}" ] && { $PERF record -e cpu-clock -F 1999 ${PERF_ARGS:-} -o $OUT/perf.data $([ -n "${PERF_THREAD:-}" ] && echo -t || echo -p) $HOST -- sleep 9 >$OUT/perf.log 2>&1 & }
bun $HERE/reload.ts $((PORT + 1)) "$F" 9000 400 2>>$OUT/host.log &
RL=$!
sleep 1
steps=(); for _ in 1 2; do steps+=(region:$BAR measure:open_load key:Control_L+l wait:600 $(type_word load 300) key:Escape wait:500); done
lat "${steps[@]}" >/dev/null
wait $RL
phase load end
sleep 2

[ -f $OUT/perf.data ] && $PERF report -i $OUT/perf.data --no-children --sort comm,dso,sym --stdio 2>/dev/null | grep -v "^#" | head -60 >$OUT/perf.txt
kill_tree $BPID; sleep 4; pkill -9 -s $BPID 2>/dev/null
kill $FX 2>/dev/null
rm -rf $OUT/p
bun $HERE/summarize.ts $OUT/host.log $OUT/lat.txt | tee $OUT/summary.txt
