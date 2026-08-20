#!/usr/bin/env bash
# Assembles a throwaway app bundle for running the AppKit host with the CEF
# engine. `nd dev` runs NDShell as a bare binary, and CEF cannot work that way:
# macOS needs the framework and five helper .app bundles at fixed paths
# relative to the executable.
#
#   swift/.build/NDShellDev.app
#   └── Contents
#       ├── Info.plist
#       ├── MacOS/NDShell                      <= the SwiftPM host, copied in
#       └── Frameworks
#           ├── Chromium Embedded Framework.framework  <= symlink to ND_CEF_ROOT
#           ├── NDShell Helper.app             <= browser_subprocess_path
#           ├── NDShell Helper (Alerts).app
#           ├── NDShell Helper (GPU).app       <= JIT entitlements
#           ├── NDShell Helper (Plugin).app
#           └── NDShell Helper (Renderer).app  <= JIT entitlements
#
# Chromium derives the four suffixed bundles from the unsuffixed one by name,
# so the names above are load-bearing. All five run the same nd-cef-helper
# binary, which does nothing but cef_execute_process.
#
# The framework is symlinked rather than copied (224 MB per rebuild otherwise),
# and the 151.3.23 distribution's own flat layout is left exactly as it is: no
# Versions/A structure is synthesized here. This bundle is never signed for
# distribution; `nd package` owns that.
#
# Prints the host executable path as its last line.
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT="$(pwd)"

CEF_VERSION="${ND_CEF_VERSION:-151.3.23-macosarm64}"
CEF_ROOT="${ND_CEF_ROOT:-$HOME/.cache/nativedesktop/cef/$CEF_VERSION}"
FRAMEWORK="$CEF_ROOT/Release/Chromium Embedded Framework.framework"
[ -d "$FRAMEWORK" ] || FRAMEWORK="$CEF_ROOT/Chromium Embedded Framework.framework"
if [ ! -x "$FRAMEWORK/Chromium Embedded Framework" ]; then
  echo "no CEF framework under $CEF_ROOT (set ND_CEF_ROOT)" >&2
  exit 1
fi

"$ROOT/scripts/mac/build-appkit-host.sh" >/dev/null
( cd swift && swift build -c release --product nd-cef-helper >/dev/null )

HOST_BIN="$ROOT/swift/.build/release/NDShell"
HELPER_BIN="${ND_CEF_HELPER:-$ROOT/swift/.build/release/nd-cef-helper}"
APP="$ROOT/swift/.build/NDShellDev.app"
NAME="NDShell"
BUNDLE_ID="dev.nativedesktop.ndshell.cefdev"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Frameworks"
cp "$HOST_BIN" "$APP/Contents/MacOS/$NAME"
ln -s "$FRAMEWORK" "$APP/Contents/Frameworks/Chromium Embedded Framework.framework"

plist() {
  # $1 = plist path, $2 = CFBundleExecutable, $3 = CFBundleIdentifier, $4 = LSUIElement
  cat >"$1" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleExecutable</key><string>$2</string>
  <key>CFBundleIdentifier</key><string>$3</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>$2</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.0.0</string>
  <key>CFBundleVersion</key><string>0.0.0</string>
  <key>LSMinimumSystemVersion</key><string>26.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
  <key>LSUIElement</key><$4/>
</dict>
</plist>
PLIST
}

plist "$APP/Contents/Info.plist" "$NAME" "$BUNDLE_ID" false

ENTITLEMENTS="$ROOT/swift/.build/nd-cef-helper.entitlements"
cat >"$ENTITLEMENTS" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>com.apple.security.cs.allow-jit</key><true/>
  <key>com.apple.security.cs.allow-unsigned-executable-memory</key><true/>
  <key>com.apple.security.cs.disable-library-validation</key><true/>
</dict>
</plist>
PLIST

# "<suffix>:<bundle id suffix>:<jit>". The empty suffix is the one CEF is
# pointed at; V8 and the GPU compiler are the two that need writable-executable
# pages.
for entry in ":default:0" " (Alerts):alerts:0" " (GPU):gpu:1" " (Plugin):plugin:0" " (Renderer):renderer:1"; do
  suffix="${entry%%:*}"
  rest="${entry#*:}"
  id_suffix="${rest%%:*}"
  jit="${rest##*:}"
  helper_name="$NAME Helper$suffix"
  helper_app="$APP/Contents/Frameworks/$helper_name.app"
  mkdir -p "$helper_app/Contents/MacOS"
  cp "$HELPER_BIN" "$helper_app/Contents/MacOS/$helper_name"
  plist "$helper_app/Contents/Info.plist" "$helper_name" "$BUNDLE_ID.helper.$id_suffix" true
  if [ "$jit" = "1" ]; then
    codesign --force --sign - --entitlements "$ENTITLEMENTS" "$helper_app" 2>/dev/null
  else
    codesign --force --sign - "$helper_app" 2>/dev/null
  fi
done

# The framework is a symlink, so the outer sign is shallow on purpose: a deep
# sign would rewrite the shared distribution in the cache.
codesign --force --sign - "$APP/Contents/MacOS/$NAME" 2>/dev/null

echo "$APP/Contents/MacOS/$NAME"
