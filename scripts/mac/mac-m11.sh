#!/usr/bin/env bash
set -euo pipefail
# scripts/mac/mac-m11.sh — M11 native-chrome acceptance on the Mac.
# Dual-mode wrapper around mac-m11-body.sh: invoked ON a Mac it runs the body
# directly; invoked from the Linux orchestrator it syncs the tree and runs the
# body over ssh (the Mac's login shell is fish, hence the explicit bash),
# then pulls the screenshots back — mirroring scripts/mac/mac-m6.sh.
cd "$(dirname "$0")/../.."

if [ "$(uname -s)" = "Darwin" ]; then
  exec ./scripts/mac/mac-m11-body.sh
fi

./scripts/mac/mac-sync.sh
ssh macbook 'bash -euo pipefail ~/nd/scripts/mac/mac-m11-body.sh'

# Bring the screenshots back for the orchestrator to view.
OUT_DIR="${ND_M11_SHOT_DIR:-/tmp}"
scp macbook:/tmp/notes-baseline.png macbook:/tmp/notes-final.png "$OUT_DIR/" 2>/dev/null || true
echo "MAC_M11_SCREENSHOTS $OUT_DIR/notes-baseline.png $OUT_DIR/notes-final.png"
