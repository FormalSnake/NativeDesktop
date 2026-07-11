#!/usr/bin/env bash
set -euo pipefail
# Sync the tree, build libnd.a + the Swift shell, run headful in the logged-in
# GUI session. $1 = example script (default counter). Runs entirely on the Mac.
SCRIPT="${1:-examples/counter/main.tsx}"
"$(dirname "$0")/mac-sync.sh"
# Mac's login shell is fish; run the remote build in bash for reliable &&/exit-code semantics.
ssh macbook "bash -euo pipefail -s" <<REMOTE
cd ~/nd
export PATH="/etc/profiles/per-user/kyandesutter/bin:\$PATH"
zig build libnd -Dbackend=abi 2>&1 | tail -3
# zig's archiver emits members that Apple's ld rejects as "not 8-byte
# aligned"; re-pack with the system ar/libtool (which normalizes alignment)
# before swiftc links it. Extracted members also land 0-permission under
# zig's ar, so chmod before libtool can even open() them.
workdir="\$(mktemp -d)"
( cd "\$workdir" && ar x ~/nd/zig-out/lib/libnd.a && chmod 644 *.o && libtool -static -o ~/nd/zig-out/lib/libnd.a *.o )
rm -rf "\$workdir"
cd swift && swift build -c release 2>&1 | tail -5
cd ~/nd && ND_SCRIPT='$SCRIPT' swift/.build/release/NDShell 2>&1 | tail -20
REMOTE
