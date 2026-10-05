#!/usr/bin/env bash
# Tab switch latency of the REAL browser app on macOS, under the CEF gate lock:
# the app starts on six fixture tabs, scripts/mac/switch-latency-drive.ts
# switches between them with real key chords and reads, for each switch, the
# first frame the tab switched to draws and the host's and child's ND_LAT hops.
# ND_SWITCH_STOCK=<browser binary> runs the same chords against that browser
# afterwards (Helium, Chrome) with a throwaway profile. Marker: ND_SWITCH_DONE.
#
# ND_APP_DIR points at the app checkout (default ~/Developer/nativebrowser).
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT="$(pwd)"

APP_DIR="${ND_APP_DIR:-$HOME/Developer/nativebrowser}"
[ -f "$APP_DIR/src/main.tsx" ] || { echo "FAIL: no app at $APP_DIR (set ND_APP_DIR)"; exit 1; }
PORT="${ND_CEF_DEBUG_PORT:-9446}"
FIXTURE_PORT="${ND_APP_FIXTURE_PORT:-9733}"
TABS="${ND_SWITCH_TABS:-6}"
HOST="$(env -u SDKROOT -u DEVELOPER_DIR ./scripts/mac/dev-cef-bundle.sh | tail -1)"

# shellcheck source=scripts/mac/cef-gate-lock.sh
. scripts/mac/cef-gate-lock.sh
HOST_PID=""
FIXTURE_PID=""
LOG=""
cleanup() {
  # ND_SWITCH_KEEP_LOG keeps the host's log past the run dir.
  if [ -n "${ND_SWITCH_KEEP_LOG:-}" ] && [ -n "${LOG:-}" ]; then cp "$LOG" "$ND_SWITCH_KEEP_LOG" 2>/dev/null || true; fi
  if [ -n "${HOST_PID:-}" ]; then kill -9 "$HOST_PID" 2>/dev/null || true; fi
  if [ -n "${FIXTURE_PID:-}" ]; then kill "$FIXTURE_PID" 2>/dev/null || true; fi
  pkill -9 -f "$ROOT/swift/.build/NDShellDev.app" 2>/dev/null || true
  cef_gate_unlock
}
trap cleanup EXIT
cef_gate_lock
PROFILE="$RUN_DIR"
mkdir -p "$PROFILE/store" "$PROFILE/cef"

bun scripts/app-chrome-fixture.ts "$FIXTURE_PORT" >"$PROFILE/fixture.log" 2>&1 &
FIXTURE_PID=$!
FIXTURE="http://127.0.0.1:$FIXTURE_PORT/"
for _ in $(seq 1 100); do curl -s --max-time 1 "$FIXTURE" >/dev/null 2>&1 && break; sleep 0.1; done

tabs="{\"id\":\"t1\",\"url\":\"$FIXTURE\",\"title\":\"\",\"pinned\":false},{\"id\":\"t2\",\"url\":\"$FIXTURE?two\",\"title\":\"\",\"pinned\":false}"
for n in $(seq 3 "$TABS"); do tabs="$tabs,{\"id\":\"t$n\",\"url\":\"$FIXTURE?tab$n\",\"title\":\"\",\"pinned\":false}"; done
cat >"$PROFILE/store/session.json" <<EOF
{"version":1,"data":{"tabs":[$tabs],"activeId":"t1","nextTabId":$((TABS + 1)),"windowWidth":1280,"windowHeight":800,"zoomByHost":{}}}
EOF
cat >"$PROFILE/store/settings.json" <<EOF
{"version":1,"data":{"searchEngine":"duckduckgo","homepage":"","restoreOnLaunch":true,"layout":"${ND_SWITCH_LAYOUT:-sidebar}","pinnedExtensions":[]}}
EOF

LOG="$PROFILE/host.log"
cd "$APP_DIR"
NATIVE_AUTOMATION=1 ND_WEBVIEW_ENGINE=chromium ND_CEF_STYLE=chrome ND_LAT_TRACE=1 \
  ND_SCRIPT="${ND_APP_SCRIPT:-src/main.tsx}" NB_STORE_DIR="$PROFILE/store" ND_CEF_CACHE="$PROFILE/cef" \
  "$HOST" "--remote-debugging-port=$PORT" --remote-allow-origins='*' >"$LOG" 2>&1 &
HOST_PID=$!
cd "$ROOT"
for _ in $(seq 1 600); do grep -q "ND_AUTOMATION_LISTENING" "$LOG" 2>/dev/null && break; sleep 0.1; done
SOCK="$(grep -m1 "ND_AUTOMATION_LISTENING" "$LOG" | sed 's/.*path=//')"
[ -n "$SOCK" ] || { echo "FAIL: no automation listener"; tail -40 "$LOG"; exit 1; }

ND_AUTOMATION_SOCKET="$SOCK" ND_CDP_PORT="$PORT" ND_ACCEPT_FIXTURE="$FIXTURE" ND_ACCEPT_TABS="$TABS" \
  ND_ACCEPT_HOST_LOG="$LOG" ND_ACCEPT_SHOTS="$PROFILE" timeout 900 bun scripts/mac/switch-latency-drive.ts
