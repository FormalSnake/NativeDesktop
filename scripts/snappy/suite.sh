#!/usr/bin/env bash
# Startup and input-to-pixels latency, one browser per call, inside rig.sh.
# Usage (from rig.sh): suite.sh <name> <kind: nb|helium> [fw-dir app-dir]
# nb: the dev-built host of fw-dir running app-dir/src, GPU through the dev
# shell's own Mesa. helium: the installed Helium on XWayland (ozone x11), so
# both get the same XTEST input and X pixel reads.
set -u
NAME=$1 KIND=$2 FW=${3:-} APPD=${4:-}
HERE=$(cd "$(dirname "$0")" && pwd)
LAT=$HERE/../latency-x11.ts
OUT=$RUN/$NAME; mkdir -p $OUT
kill_tree() { local c; for c in $(pgrep -P "$1" 2>/dev/null); do kill_tree "$c"; done; kill "$1" 2>/dev/null || true; }
PORT=$((9900 + RANDOM % 50))
bun $HERE/fixture.ts $PORT >$OUT/fixture.log 2>&1 & FX=$!
sleep 1
F=http://127.0.0.1:$PORT/

seed() {
  rm -rf $OUT/p; mkdir -p $OUT/p/store $OUT/p/data $OUT/p/cache $OUT/p/xdg; chmod 700 $OUT/p/xdg
  T='{"id":"t1","url":"'$F'","title":"","pinned":false}'
  echo '{"version":1,"data":{"tabs":['$T'],"activeId":"t1","nextTabId":2,"windowWidth":1920,"windowHeight":1080,"zoomByHost":{}}}' > $OUT/p/store/session.json
  echo '{"version":1,"data":{"searchEngine":"duckduckgo","homepage":"","restoreOnLaunch":true,"layout":"sidebar","pinnedExtensions":[]}}' > $OUT/p/store/settings.json
}

launch() {
  START=$(date +%s%3N)
  if [ "$KIND" = nb ]; then
    ( cd $APPD
      exec env LD_LIBRARY_PATH=$(dirname $ND_PIN_DRI_PATH):$ND_CEF_LD_LIBRARY_PATH:$LD_LIBRARY_PATH \
        __EGL_VENDOR_LIBRARY_FILENAMES=$ND_PIN_EGL_VENDOR_FILE LIBGL_DRIVERS_PATH=$ND_PIN_DRI_PATH GBM_BACKENDS_PATH=$ND_PIN_GBM_PATH \
        ND_CEF_ROOT=$HOME/.cache/nativedesktop/cef/151.3.23-linux64/Release \
        XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR XDG_DATA_HOME=$OUT/p/data ND_CEF_CACHE=$OUT/p/cache NB_STORE_DIR=$OUT/p/store \
        GDK_BACKEND=x11 NATIVE_AUTOMATION=1 ND_WEBVIEW_ENGINE=chromium ND_CEF_STYLE=chrome ND_SCRIPT=${NB_SCRIPT:-src/main.tsx} \
        ND_APP_ID=dev.snappy.$NAME ${NB_ENV:-} setsid $FW/zig-out/bin/nd-hello --remote-debugging-port=$((PORT + 1)) --remote-allow-origins='*' ) >>$OUT/host.log 2>&1 &
  else
    ( exec env -u LD_LIBRARY_PATH NIX_LD=/run/current-system/sw/share/nix-ld/lib/ld.so NIX_LD_LIBRARY_PATH=/run/current-system/sw/share/nix-ld/lib \
        setsid ${HELIUM:-helium} --user-data-dir=$OUT/p/data --no-first-run --no-default-browser-check \
        --password-store=basic --ozone-platform=x11 --enable-features=AcceleratedVideoDecodeLinuxGL --remote-debugging-port=$((PORT + 1)) "$F" ) >>$OUT/host.log 2>&1 &
  fi
  BPID=$!
}

stop() { kill_tree $BPID; sleep 4; pkill -9 -s $BPID 2>/dev/null; sleep 1; }

# The fixture page is one flat colour: its first paint is that pixel turning.
PAGE=1300,700
COLOR=2a6fdb

seed
for run in cold warm1 warm2; do
  launch
  ND_LAT_START_EPOCH_MS=$START timeout 90 bun $LAT $BPID startup:0,0,400,300 until:page_painted:$PAGE:$COLOR 2>&1 | sed "s/^ND_LAT /ND_LAT $run./" | tee -a $OUT/lat.txt
  sleep 6
  [ $run = warm2 ] || stop
done

# Input latency against the running warm instance.
if [ "$KIND" = nb ]; then BAR=560,60,800,420; else BAR=0,0,1920,130; fi
steps=()
for i in 1 2 3 4 5; do
  steps+=(region:$BAR measure:open_ctrl_l key:Control_L+l wait:700 measure:type_1 key:g wait:400 measure:type_2 key:i wait:400 measure:type_3 key:t wait:400 measure:backspace key:BackSpace wait:500 key:Escape wait:700)
  steps+=(region:$BAR measure:open_ctrl_t key:Control_L+t wait:800 measure:type_t1 key:h wait:400 measure:type_t2 key:e wait:400 key:Escape wait:300 key:Escape wait:900)
done
timeout 300 bun $LAT $BPID "${steps[@]}" 2>&1 | tee -a $OUT/lat.txt | tail -3
[ -n "${HYPRLAND_INSTANCE_SIGNATURE:-}" ] && grim $OUT/end.png 2>/dev/null
stop
kill $FX 2>/dev/null
rm -rf $OUT/p
