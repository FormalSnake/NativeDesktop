#!/usr/bin/env bash
# Native messaging on the Chromium engine: a fixture host whose manifest sits
# only where Chrome looks ($XDG_CONFIG_HOME/google-chrome) has to answer the
# fixture extension, and a manifest already in the host's own
# NativeMessagingHosts has to survive the start. Both XDG homes are the run's
# own, so no real browser directory is read or written.
# Marker: ND_CEF_NATIVE_MESSAGING_OK.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"

: "${ND_CEF_DIST:=$HOME/.cache/nativedesktop/cef/151.3.23-linux64}"
[ -f "$ND_CEF_DIST/Release/libcef.so" ] || { echo "SKIP: no CEF distribution at $ND_CEF_DIST"; exit 0; }
# Same stitching as scripts/headless-webview-cef-chrome.sh.
ln -sfn "$ND_CEF_DIST"/Resources/* "$ND_CEF_DIST/Release/"
export ND_CEF_ROOT="$ND_CEF_DIST/Release"

if [ -n "${ND_CEF_LD_LIBRARY_PATH:-}" ]; then
  export LD_LIBRARY_PATH="$ND_CEF_LD_LIBRARY_PATH${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
export DISPLAY="${ND_CEF_DISPLAY:-:96}"
export GDK_BACKEND=x11
export GSK_RENDERER=cairo
export NATIVE_AUTOMATION=1
export ND_WEBVIEW_ENGINE=chromium
export ND_CEF_STYLE=chrome
export ND_APP_ID="${ND_APP_ID:-dev.nativedesktop.headlessNativeMessaging}"
CDP_PORT="${ND_CDP_PORT:-9334}"
export ND_CDP_PORT="$CDP_PORT"

RUN_DIR="$(mktemp -d)"
export XDG_DATA_HOME="$RUN_DIR/data"
export XDG_CONFIG_HOME="$RUN_DIR/config"
export ND_CEF_CACHE="$RUN_DIR/cef"
CHROME_HOSTS="$XDG_CONFIG_HOME/google-chrome/NativeMessagingHosts"
OWN_HOSTS="$ND_CEF_CACHE/NativeMessagingHosts"
mkdir -p "$CHROME_HOSTS" "$OWN_HOSTS"
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
echo '{"own":true}' >"$OWN_HOSTS/dev.nativedesktop.keep.json"

XVFB_PID=""
HOST_PID=""
if [ -z "${ND_CEF_KEEP_DISPLAY:-}" ]; then
  Xvfb "$DISPLAY" -screen 0 1280x900x24 -nolisten tcp >/dev/null 2>&1 &
  XVFB_PID=$!
fi
trap '[ -n "$HOST_PID" ] && kill "$HOST_PID" 2>/dev/null; [ -n "$XVFB_PID" ] && kill "$XVFB_PID" 2>/dev/null; rm -rf "$RUN_DIR"; true' EXIT
for _ in $(seq 1 100); do
  xwininfo -root >/dev/null 2>&1 && break
  sleep 0.1
done
xwininfo -root >/dev/null 2>&1 || { echo "FAIL: no X server on $DISPLAY"; exit 1; }

LOG="$RUN_DIR/host.log"
BUN_OPTIONS="$(bun scripts/bun-options.ts examples/webview-probe/cef-native-messaging.tsx)" ND_SCRIPT=examples/webview-probe/cef-native-messaging.tsx ./zig-out/bin/nd-hello \
  --load-extension="$ROOT/scripts/fixtures/native-messaging/ext" \
  --remote-debugging-port="$CDP_PORT" --remote-allow-origins='*' >"$LOG" 2>&1 &
HOST_PID=$!
for _ in $(seq 1 600); do
  grep -q "ND_WEBVIEW_ENGINE chromium" "$LOG" && break
  sleep 0.1
done
grep -q "ND_WEBVIEW_ENGINE chromium" "$LOG" || { echo "FAIL: the chromium engine did not load"; tail -40 "$LOG"; exit 1; }

FAILED=0
bun scripts/native-messaging-drive.ts || FAILED=1
grep -m1 "ND_CEF_NATIVE_MESSAGING" "$LOG" || true
grep -i "native messaging" "$LOG" | head -5 || true
if [ "$(cat "$OWN_HOSTS/dev.nativedesktop.keep.json")" != '{"own":true}' ]; then
  echo "FAIL: a manifest already in the host's NativeMessagingHosts was replaced"
  FAILED=1
fi
[ "$FAILED" -eq 0 ] || { echo "cef native messaging: FAILED"; exit 1; }
echo "ND_CEF_NATIVE_MESSAGING_OK"
