#!/usr/bin/env bash
# Copies the dev CEF bundle (scripts/mac/dev-cef-bundle.sh) to
# swift/.build/NDShellSigned.app and signs it the way `nd package` ships one
# (packages/nd/src/package/mac.ts): inside out, hardened runtime, JIT
# entitlements on the renderer and GPU helpers, a Developer ID from the keychain
# unless APPLE_SIGN_IDENTITY names one. The framework is copied, not linked, so
# signing never touches the shared distribution in the cache. For checks that
# depend on who signed the browser, such as 1Password's browser verification.
#
# Prints the signed host executable path as its last line.
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT="$(pwd)"

IDENTITY="${APPLE_SIGN_IDENTITY:-$(security find-identity -v -p codesigning | awk -F'"' '/Developer ID Application/ {print $2; exit}')}"
[ -n "$IDENTITY" ] || { echo "no Developer ID Application identity in the keychain (set APPLE_SIGN_IDENTITY)" >&2; exit 1; }

DEV_HOST="$(env -u SDKROOT -u DEVELOPER_DIR ./scripts/mac/dev-cef-bundle.sh | tail -1)"
DEV_APP="${DEV_HOST%/Contents/MacOS/*}"
APP="$ROOT/swift/.build/NDShellSigned.app"
FW_NAME="Chromium Embedded Framework.framework"

rm -rf "$APP"
mkdir -p "$APP"
# -P keeps the framework's own internal links as links, which codesign needs.
cp -RP "$DEV_APP/Contents" "$APP/"
rm "$APP/Contents/Frameworks/$FW_NAME"
cp -RP "$(readlink "$DEV_APP/Contents/Frameworks/$FW_NAME")" "$APP/Contents/Frameworks/$FW_NAME"

APP_ENT="$ROOT/swift/.build/nd-signed-app.entitlements"
HELPER_ENT="$ROOT/swift/.build/nd-cef-helper.entitlements"
cat >"$APP_ENT" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>com.apple.security.cs.allow-jit</key><true/>
  <key>com.apple.security.cs.allow-unsigned-executable-memory</key><true/>
</dict>
</plist>
PLIST

sign() { codesign --force --options runtime --timestamp=none --sign "$IDENTITY" "$@" >/dev/null; }
FW="$APP/Contents/Frameworks/$FW_NAME"
for lib in "$FW"/Libraries/*.dylib; do sign "$lib"; done
sign "$FW"
for helper in "$APP/Contents/Frameworks/"*Helper*.app; do
  name="$(basename "$helper" .app)"
  case "$name" in
    *"(GPU)"|*"(Renderer)") sign --entitlements "$HELPER_ENT" "$helper/Contents/MacOS/$name"; sign --entitlements "$HELPER_ENT" "$helper" ;;
    *) sign "$helper/Contents/MacOS/$name"; sign "$helper" ;;
  esac
done
sign --entitlements "$APP_ENT" "$APP/Contents/MacOS/NDShell"
sign --entitlements "$APP_ENT" "$APP"
codesign --verify --strict "$APP"
echo "signed with '$IDENTITY'" >&2
echo "$APP/Contents/MacOS/NDShell"
