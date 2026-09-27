#!/usr/bin/env bash
# Native messaging on the Chromium engine: a fixture host whose manifest sits
# only where Chrome looks (~/Library/Application Support/Google/Chrome) has to
# answer the fixture extension, and a manifest already in the host's own
# NativeMessagingHosts has to survive the start. HOME points at a fixture home
# so no real browser directory is read or written.
# Marker: ND_CEF_NATIVE_MESSAGING_MAC_OK.
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT="$(pwd)"

PORT="${ND_CEF_DEBUG_PORT:-9601}"
HOST="$(env -u SDKROOT -u DEVELOPER_DIR ./scripts/mac/dev-cef-bundle.sh | tail -1)"
# shellcheck source=scripts/mac/cef-gate-lock.sh
. scripts/mac/cef-gate-lock.sh
HOST_PID=""
cleanup() {
  if [ -n "${HOST_PID:-}" ]; then kill -9 "$HOST_PID" 2>/dev/null || true; fi
  cef_gate_unlock
}
trap cleanup EXIT
cef_gate_lock

FIXTURE_HOME="$RUN_DIR/home"
CHROME_HOSTS="$FIXTURE_HOME/Library/Application Support/Google/Chrome/NativeMessagingHosts"
CACHE="$RUN_DIR/cef"
mkdir -p "$CHROME_HOSTS" "$CACHE/NativeMessagingHosts"
cat >"$CHROME_HOSTS/dev.nativedesktop.echo.json" <<JSON
{
  "name": "dev.nativedesktop.echo",
  "description": "ND gate echo host",
  "path": "$ROOT/scripts/fixtures/native-messaging/echo-host.sh",
  "type": "stdio",
  "allowed_origins": ["chrome-extension://poekbdjmocmcnaipbndfoomkigdelckf/"]
}
JSON
echo '{"chrome":true}' >"$CHROME_HOSTS/dev.nativedesktop.keep.json"
echo '{"own":true}' >"$CACHE/NativeMessagingHosts/dev.nativedesktop.keep.json"

LOG="$RUN_DIR/host.log"
HOME="$FIXTURE_HOME" NATIVE_AUTOMATION=1 ND_WEBVIEW_ENGINE=chromium ND_CEF_STYLE=chrome \
  ND_CEF_CACHE="$CACHE" ND_SCRIPT=examples/webview-probe/cef-native-messaging.tsx "$HOST" \
  "--load-extension=$ROOT/scripts/fixtures/native-messaging/ext" \
  "--remote-debugging-port=$PORT" --remote-allow-origins='*' >"$LOG" 2>&1 &
HOST_PID=$!
for _ in $(seq 1 400); do
  grep -q "ND_WEBVIEW_ENGINE chromium" "$LOG" && break
  sleep 0.1
done
grep -q "ND_WEBVIEW_ENGINE chromium" "$LOG" || { echo "FAIL: the chromium engine did not load"; tail -20 "$LOG"; exit 1; }

FAILED=0
ND_CDP_PORT="$PORT" bun scripts/native-messaging-drive.ts || FAILED=1
grep -m1 "ND_CEF_NATIVE_MESSAGING" "$LOG" || true
grep "native messaging" "$LOG" | head -5 || true
if [ "$(cat "$CACHE/NativeMessagingHosts/dev.nativedesktop.keep.json")" != '{"own":true}' ]; then
  echo "FAIL: a manifest already in the host's NativeMessagingHosts was replaced"
  FAILED=1
fi
[ "$FAILED" -eq 0 ] || { echo "cef native messaging: FAILED"; exit 1; }
echo "ND_CEF_NATIVE_MESSAGING_MAC_OK"
