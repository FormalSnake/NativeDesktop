#!/usr/bin/env bash
# scripts/gettree-soak.sh [cycles] — the first-getTree crash gate for the GTK
# host. Launches examples/notes, waits for the automation listener + first
# commit, issues ONE getTree, and asserts both a well-formed tree and a
# still-alive host; repeats `cycles` times (default 50). Every cycle must be
# clean. Baseline before the vtCreate ref_sink + release_node fix: ~33/50
# (the core's handle table was unowned, so a single-child container swap left
# a dangling entry the first getTree's node_visible/a11y probes dereferenced).
# Runs against any GTK display backend, including headful Quartz on macOS —
# windows opening and closing per cycle is expected there.
set -euo pipefail
cd "$(dirname "$0")/.."

CYCLES="${1:-50}"
HOST=./zig-out/bin/nd-hello
[ -x "$HOST" ] || { echo "FAIL: $HOST missing (run zig build)"; exit 1; }

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
export NATIVE_AUTOMATION=1

WORK=$(mktemp -d)
HOST_PID=""
trap 'kill "$HOST_PID" 2>/dev/null || true; rm -rf "$WORK"' EXIT

pass=0
for i in $(seq 1 "$CYCLES"); do
  LOG="$WORK/host-$i.log"
  ND_SCRIPT=examples/notes/main.tsx "$HOST" >"$LOG" 2>&1 &
  HOST_PID=$!

  ready=0
  for _ in $(seq 1 120); do
    if grep -q "ND_AUTOMATION_LISTENING" "$LOG" && grep -q "ND_COMMIT_APPLIED" "$LOG"; then
      ready=1
      break
    fi
    kill -0 "$HOST_PID" 2>/dev/null || break
    sleep 0.1
  done

  ok=0
  if [ "$ready" = 1 ]; then
    SOCK=$(grep -m1 "ND_AUTOMATION_LISTENING" "$LOG" | sed 's/.*path=//')
    if ND_AUTOMATION_SOCKET="$SOCK" bun -e '
      const { AutomationClient } = await import("@nativedesktop/test");
      const client = await AutomationClient.connect();
      const tree = await Promise.race([
        client.call("getTree"),
        new Promise((_, reject) => setTimeout(() => reject(new Error("getTree timed out")), 10_000)),
      ]);
      if (!tree || typeof tree.root?.ref !== "number") throw new Error("malformed tree");
      client.close();
    ' >"$WORK/drive-$i.log" 2>&1 && kill -0 "$HOST_PID" 2>/dev/null; then
      ok=1
    fi
  fi

  if [ "$ok" = 1 ]; then
    pass=$((pass + 1))
  else
    echo "cycle $i: FAIL"
    tail -5 "$WORK/drive-$i.log" 2>/dev/null || true
    tail -5 "$LOG"
  fi

  kill "$HOST_PID" 2>/dev/null || true
  wait "$HOST_PID" 2>/dev/null || true
  HOST_PID=""
done

echo "gettree soak: $pass/$CYCLES clean"
if [ "$pass" -eq "$CYCLES" ]; then
  echo "ND_GETTREE_SOAK_OK"
else
  exit 1
fi
