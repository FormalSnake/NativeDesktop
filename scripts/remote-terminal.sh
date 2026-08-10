#!/usr/bin/env bash
set -euo pipefail
# scripts/remote-terminal.sh [appkit|gtk] — thin wrapper around
# scripts/remote-terminal-drive.ts, which (via @nativedesktop/test) now owns
# the whole flow itself: starting the fake byte-plane server, launching the
# remote-terminal example with NATIVE_AUTOMATION=1 and the matching
# ND_REMOTE_* env, and driving the assertions. This wrapper exists only so
# `scripts/remote-terminal.sh` stays a stable entry point (docs, CI) and so a
# GTK run gets the nix devshell it needs on PATH.
#
# GTK builds/runs need the nix devshell (pkg-config + brew GTK) — wrap the
# whole call in `nix develop --command`. AppKit needs only zig+swift on PATH.
cd "$(dirname "$0")/.."
BACKEND="${1:-appkit}"

if [ "$BACKEND" = gtk ]; then
  zig build >/dev/null 2>&1 || { echo "FAIL: zig build (needs nix devshell for GTK)"; exit 1; }
else
  zig build libnd -Dbackend=abi >/dev/null 2>&1
  ( cd swift && env -u SDKROOT -u DEVELOPER_DIR swift build >/dev/null 2>&1 )
fi

bun scripts/remote-terminal-drive.ts "$BACKEND"
