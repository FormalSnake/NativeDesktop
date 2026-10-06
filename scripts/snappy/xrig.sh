#!/usr/bin/env bash
# Xvfb + openbox stand-in for rig.sh (no GPU): xrig.sh <run-name> <inner.sh> [args]
set -u
RUN="${SNAPPY_RUNS:-$HOME/snappy-runs}/$1"; shift; INNER=$1; shift; mkdir -p "$RUN"
DNUM=$((100 + RANDOM % 50)); Xvfb :$DNUM -screen 0 1920x1080x24 -nolisten tcp >$RUN/xvfb.log 2>&1 & XV=$!
sleep 1; export DISPLAY=:$DNUM; openbox >/dev/null 2>&1 & OB=$!; sleep 1
export XDG_RUNTIME_DIR=$(mktemp -d /tmp/ndx.XXXXXX); chmod 700 $XDG_RUNTIME_DIR
export RUN
bash "$INNER" "$@"
kill $OB $XV 2>/dev/null; rm -rf $XDG_RUNTIME_DIR
