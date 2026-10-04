#!/usr/bin/env bash
# Content-blocking gate for the Chromium engine on macOS: runs
# examples/adblock-probe headful and asserts each of its three phases from the
# ND_ADBLOCK_PHASE lines it prints. Marker: ND_ADBLOCK_OK.
set -euo pipefail
cd "$(dirname "$0")/../.."

HOST="$(./scripts/mac/dev-cef-bundle.sh | tail -1)"
HOST_PID=""
# shellcheck source=scripts/mac/cef-gate-lock.sh
. scripts/mac/cef-gate-lock.sh
cleanup() {
  if [ -n "${HOST_PID:-}" ]; then kill -9 "$HOST_PID" 2>/dev/null || true; fi
  cef_gate_unlock
}
trap cleanup EXIT
cef_gate_lock
LOG="${ND_ADBLOCK_LOG:-$RUN_DIR/host.log}"

NATIVE_AUTOMATION=1 ND_WEBVIEW_ENGINE=chromium ND_CEF_STYLE="${ND_CEF_STYLE:-chrome}" ND_WEBVIEW_TRACE=1 \
  ND_CEF_CACHE="${ND_CEF_CACHE:-$RUN_DIR/cef}" ND_SCRIPT=examples/adblock-probe/main.tsx "$HOST" >"$LOG" 2>&1 &
HOST_PID=$!
for _ in $(seq 1 600); do
  grep -qE "ND_ADBLOCK_PROBE_DONE" "$LOG" && break
  kill -0 "$HOST_PID" 2>/dev/null || break
  sleep 0.1
done
grep -q "ND_ADBLOCK_PROBE_DONE" "$LOG" || {
  echo "FAIL: the probe did not finish"
  grep -E "ND_ADBLOCK|ND_WARN|ND_WEBVIEW_ENGINE" "$LOG" | tail -20
  exit 1
}
bun scripts/adblock-verdict.ts "$LOG"
