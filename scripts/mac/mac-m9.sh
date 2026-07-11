#!/usr/bin/env bash
set -euo pipefail
# scripts/mac/mac-m9.sh — M9 acceptance on the Mac: sync -> build -> `nd
# package mac` -> assert the ad-hoc-signed .app (+ allow-jit) -> launch the
# packaged .app headful-in-session -> run the update-verify/download/stage
# flow against a local loopback server. Mirrors mac-m6.sh's structure
# (bash heredoc over ssh; Mac login shell is fish). Screenshots aren't part
# of this leg — it's a pass/fail packaging + update gate, not a visual drive.
cd "$(dirname "$0")/../.."
"$(dirname "$0")/mac-sync.sh"

ssh macbook 'bash -euo pipefail -s' <<'REMOTE'
export PATH="/etc/profiles/per-user/kyandesutter/bin:/opt/homebrew/bin:$PATH"
cd ~/nd
zig build libnd -Dbackend=abi >/dev/null 2>&1
# Repack libnd.a for Apple ld (ar/libtool recipe — zig's members are rejected).
workdir="$(mktemp -d)"; ( cd "$workdir" && ar x ~/nd/zig-out/lib/libnd.a && chmod 644 *.o && libtool -static -o ~/nd/zig-out/lib/libnd.a *.o ); rm -rf "$workdir"
(cd swift && swift build -c release >/dev/null 2>&1)
zig build update-verify >/dev/null 2>&1
bun install --frozen-lockfile >/dev/null 2>&1

# packageMac() assembles into dist/mac/Gallery.app without cleaning it first
# (not idempotent against a leftover .app from a prior run) — cpSync then
# throws "src and dest cannot be the same". dist/ is gitignored, ephemeral
# build output; start every gate run from a clean slate (same fix as the
# Linux leg, scripts/headless-m9.sh).
rm -rf dist

# 1. Package + deep-sign the .app (ad-hoc; notarize skipped — no creds here).
ND_APP_VERSION=0.9.0 bun tools/package.ts mac >/tmp/mac-pkg.log 2>&1 || { echo "FAIL package"; cat /tmp/mac-pkg.log; exit 1; }
cat /tmp/mac-pkg.log
grep -q ND_PACKAGE_APP_SIGNED /tmp/mac-pkg.log || { echo "FAIL not signed"; exit 1; }
grep -q ND_PACKAGE_NOTARIZE_SKIPPED /tmp/mac-pkg.log || { echo "FAIL notarize gate marker missing"; exit 1; }
codesign --verify dist/mac/Gallery.app && echo "MAC_M9_CODESIGN_OK"
codesign -d --entitlements - dist/mac/Gallery.app 2>&1 | grep -q allow-jit && echo "MAC_M9_ALLOW_JIT_OK"
PUB=$(grep -m1 ND_PACKAGE_MANIFEST /tmp/mac-pkg.log | sed 's/.*pub=//')

# 2. Launch the packaged .app headful-in-session; assert it presents a commit.
# examples/counter (not gallery): gallery's <select options={fruits}> passes
# an array-valued prop, which the binary NDP encoder (src/runtime.zig
# unconditionally advertises "binary" in HelloAck) cannot represent (spec
# §5.3 has no array/object tag) — a pre-existing core/runtime gap, not a
# packaging defect. counter has no array/object props (matches T4's own
# packaged-launch smoke check and the sibling mac-m6.sh legs).
launchctl print "gui/$(id -u)" >/dev/null 2>&1 || { echo "FAIL: no GUI session for uid $(id -u)"; exit 1; }
ND_SCRIPT=examples/counter/main.tsx dist/mac/Gallery.app/Contents/MacOS/NDShell >/tmp/mac-app.log 2>&1 &
APP_PID=$!
for _ in $(seq 1 100); do grep -q ND_COMMIT_APPLIED /tmp/mac-app.log && break; sleep 0.1; done
grep -q ND_COMMIT_APPLIED /tmp/mac-app.log || { echo "FAIL packaged .app did not present"; cat /tmp/mac-app.log; kill "$APP_PID" 2>/dev/null; exit 1; }
kill -TERM "$APP_PID" 2>/dev/null || true; wait "$APP_PID" 2>/dev/null || true
echo "MAC_M9_LAUNCH_OK"

# 3. Update flow on the Mac (zero-network, loopback server).
bun tools/update-server.ts dist/update 0 >/tmp/mac-srv.log 2>&1 &
SRV=$!; for _ in $(seq 1 50); do grep -q ND_UPDATE_SERVER_LISTENING /tmp/mac-srv.log && break; sleep 0.1; done
PORT=$(grep -m1 ND_UPDATE_SERVER_LISTENING /tmp/mac-srv.log | sed 's/.*port=//')
mkdir -p /tmp/mac-m9-stage
ND_APP_VERSION=0.9.0 bun scripts/m9-drive.ts "http://127.0.0.1:$PORT" "$PUB" /tmp/mac-m9-stage 2>&1 | tail -4
kill "$SRV" 2>/dev/null || true
echo "MAC_M9_OK"
REMOTE
echo "MAC_M9_DONE"
