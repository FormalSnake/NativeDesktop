#!/usr/bin/env bash
set -euo pipefail
# scripts/mac/mac-m6.sh — the M6b acceptance drive, headful-in-session on the
# Mac (GUI uid via $(id -u); probe fact: 502). Mirrors headless-m4.sh (counter
# click x3 + waitFor + screenshot + D11 SIGSTOP SLO) and headless-m5c.sh
# (gallery widget-create + ListView itemCount asserts), plus the kill9
# equivalent (child killed -> window survives -> overlay appears). Everything
# runs ON the Mac over ssh (bash heredoc — the Mac's login shell is fish);
# screenshots come back via scp for the orchestrator to view.
cd "$(dirname "$0")/../.."
"$(dirname "$0")/mac-sync.sh"

ssh macbook 'bash -euo pipefail -s' <<'REMOTE'
export PATH="/etc/profiles/per-user/kyandesutter/bin:$PATH"
cd ~/nd
launchctl print "gui/$(id -u)" >/dev/null 2>&1 || { echo "FAIL: no GUI session for uid $(id -u)"; exit 1; }

zig build libnd -Dbackend=abi >/dev/null 2>&1
# zig's archiver emits members Apple's ld rejects ("not 8-byte aligned") and
# extracts them 0-permission; repack with the system ar/libtool before the
# Swift link (same recipe as scripts/mac/mac-run.sh).
workdir="$(mktemp -d)"
( cd "$workdir" && ar x ~/nd/zig-out/lib/libnd.a && chmod 644 *.o && libtool -static -o ~/nd/zig-out/lib/libnd.a *.o )
rm -rf "$workdir"
( cd swift && swift build -c release >/dev/null 2>&1 )

pkill -f 'swift/.build/release/NDShell' 2>/dev/null || true

run_leg() {  # $1=app script  $2=mode(counter|gallery)  $3=screenshot out path
  local LOG="/tmp/nd-m6-$2.log"
  rm -f "$LOG" "$3"
  ND_SCRIPT="$1" NATIVE_AUTOMATION=1 swift/.build/release/NDShell >"$LOG" 2>&1 &
  local PID=$!
  for _ in $(seq 1 120); do
    grep -q ND_AUTOMATION_LISTENING "$LOG" && grep -q ND_COMMIT_APPLIED "$LOG" && break
    sleep 0.1
  done
  grep -q ND_AUTOMATION_LISTENING "$LOG" || { echo "FAIL $2: no automation listener"; cat "$LOG"; kill "$PID" 2>/dev/null; exit 1; }
  grep -q ND_COMMIT_APPLIED "$LOG" || { echo "FAIL $2: no commit applied"; cat "$LOG"; kill "$PID" 2>/dev/null; exit 1; }
  local SOCK
  SOCK=$(grep -m1 ND_AUTOMATION_LISTENING "$LOG" | sed 's/.*path=//')

  ND_AUTOMATION_SOCKET="$SOCK" ND_SHOT_PATH="$3" bun scripts/m6-drive.ts "$2" >"/tmp/nd-m6-drive-$2.log" 2>&1 \
    || { echo "FAIL drive $2"; cat "/tmp/nd-m6-drive-$2.log"; cat "$LOG"; kill "$PID" 2>/dev/null; exit 1; }
  cat "/tmp/nd-m6-drive-$2.log"
  file "$3" | grep -q "PNG image" || { echo "FAIL $2: not a png"; kill "$PID" 2>/dev/null; exit 1; }

  if [ "$2" = "counter" ]; then
    # D11 SLO leg: stall the bun child; automation must still answer <5s.
    # Find it via the process tree (pgrep -P), never machine-wide.
    local BUN
    BUN=$(pgrep -P "$PID" -f bun | head -1)
    [ -n "$BUN" ] || { echo "FAIL: no bun child for SLO"; kill "$PID" 2>/dev/null; exit 1; }
    kill -STOP "$BUN"
    local SLO_RC=0
    ND_AUTOMATION_SOCKET="$SOCK" ND_SHOT_PATH=/tmp/nd-m6-slo.png timeout 5 bun scripts/m6-drive.ts --slo >/tmp/nd-m6-slo.log 2>&1 || SLO_RC=$?
    kill -CONT "$BUN"
    [ "$SLO_RC" -eq 0 ] || { echo "FAIL: SLO >5s while child stalled"; cat /tmp/nd-m6-slo.log; kill "$PID" 2>/dev/null; exit 1; }
    cat /tmp/nd-m6-slo.log
    grep -q M6_SLO_OK /tmp/nd-m6-slo.log || { echo "FAIL: SLO no marker"; kill "$PID" 2>/dev/null; exit 1; }

    # kill9 equivalent: child killed -> host survives -> overlay appears
    # (marker parity with scripts/kill9-test.sh; overlay NOT dev-gated).
    BUN=$(pgrep -P "$PID" -f bun | head -1)
    [ -n "$BUN" ] || { echo "FAIL: no bun child for kill9"; kill "$PID" 2>/dev/null; exit 1; }
    kill -9 "$BUN"
    for _ in $(seq 1 30); do grep -q ND_CHILD_EXITED "$LOG" && break; sleep 0.1; done
    grep -q ND_CHILD_EXITED "$LOG" || { echo "FAIL: host did not report child exit"; cat "$LOG"; kill "$PID" 2>/dev/null; exit 1; }
    sleep 3
    kill -0 "$PID" 2>/dev/null || { echo "FAIL: host died with the child"; cat "$LOG"; exit 1; }
    for _ in $(seq 1 30); do grep -q ND_OVERLAY_SHOWN "$LOG" && break; sleep 0.1; done
    grep -q ND_OVERLAY_SHOWN "$LOG" || { echo "FAIL: no overlay after child death"; cat "$LOG"; kill "$PID" 2>/dev/null; exit 1; }
    echo "M6_KILL9_OK window survived child death, overlay shown"
  fi

  kill -TERM "$PID" 2>/dev/null || true
  wait "$PID" 2>/dev/null || true
}

run_leg examples/counter/main.tsx counter /tmp/nd-m6-counter.png
run_leg examples/gallery/main.tsx gallery /tmp/nd-m6-gallery.png
echo "MAC_M6_OK"
REMOTE

# Bring the screenshots back for the orchestrator to view.
OUT_DIR="${ND_M6_SHOT_DIR:-/tmp}"
scp macbook:/tmp/nd-m6-counter.png macbook:/tmp/nd-m6-gallery.png macbook:/tmp/nd-m6-slo.png "$OUT_DIR/" 2>/dev/null || true
echo "MAC_M6_SCREENSHOTS $OUT_DIR/nd-m6-counter.png $OUT_DIR/nd-m6-gallery.png $OUT_DIR/nd-m6-slo.png"
