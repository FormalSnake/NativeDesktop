#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

# Inside the repo's nix devshell, a stale SDKROOT/DEVELOPER_DIR breaks the
# system Swift toolchain -- unset both so `swift build` uses Xcode's own.
env -u SDKROOT -u DEVELOPER_DIR swift build -c release

# Install to a VISIBLE directory (bin/, not the dot-hidden .build/) — the
# System Settings > Screen Recording file picker hides dot-folders, so the
# grantable copy must live somewhere Finder can reach. A stable install path
# also gives TCC a consistent identity anchor. Skip the install entirely when
# the bytes are unchanged: the existing bin/ndshot may carry a certificate
# signature (grantable across rebuilds) that a pointless re-copy + re-sign
# from a keychain-less context (agent/CI) would downgrade to ad-hoc.
mkdir -p bin
bin="bin/ndshot"
if [ -f "$bin" ] && cmp -s .build/release/ndshot "$bin".unsigned 2>/dev/null; then
  echo "ndshot unchanged; keeping existing signed $(cd bin && pwd)/ndshot"
  exit 0
fi
cp -f .build/release/ndshot "$bin".unsigned
cp -f .build/release/ndshot "$bin"

# Sign with a REAL identity when the keychain has one (Developer ID
# preferred): a certificate-backed signature gives the binary an
# identifier-based designated requirement, so the TCC Screen Recording grant
# SURVIVES rebuilds. Ad-hoc (-) works but is content-hash-keyed — every
# rebuild that changes the bytes needs a re-grant in System Settings.
# Identity signing needs keychain access, which only a GUI session can
# approve — from agent/SSH contexts it fails with errSecInternalComponent,
# so fall back to ad-hoc with a loud warning.
identity=$(security find-identity -v -p codesigning 2>/dev/null | awk -F'"' '/Developer ID Application/ {print $2; exit}')
if [ -z "$identity" ]; then
  identity=$(security find-identity -v -p codesigning 2>/dev/null | awk -F'"' '/Apple Development/ {print $2; exit}')
fi
if [ -n "$identity" ] && codesign -f -s "$identity" -i com.nativedesktop.ndshot "$bin" 2>/dev/null; then
  echo "ndshot built, signed with '$identity': $(cd bin && pwd)/ndshot"
else
  codesign -f -s - -i com.nativedesktop.ndshot "$bin"
  echo "WARNING: identity signing unavailable (locked keychain / no GUI session) — ad-hoc signed." >&2
  echo "WARNING: the Screen Recording grant will break on the next binary change; run" >&2
  echo "WARNING: tools/ndshot/build.sh once from a normal terminal to get a durable signature." >&2
  echo "ndshot built, ad-hoc signed: $(cd bin && pwd)/ndshot"
fi
