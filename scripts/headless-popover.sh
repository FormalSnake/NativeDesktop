#!/usr/bin/env bash
# scripts/headless-popover.sh: the Linux half of the <popover anchorRef> gate.
# Runs examples/popoveranchor under weston-headless and drives it with
# scripts/popover-anchor-drive.ts, the same script the AppKit leg runs.
#
# What it proves on GTK: a popover portalled into the off-window pool has no
# tree parent at all, so `anchorRef` is the only thing that can place it, and
# getTree's geometry puts the panel against the trigger button once it opens.
# Dropping the ref is asserted through a different gesture per backend (the
# drive branches on ND_BACKEND): GTK cannot synthesize the Escape keystroke
# that closes the panel first, so it drops the ref while the panel is up.
set -euo pipefail
cd "$(dirname "$0")/.."

exec ./scripts/headless-run.sh \
  examples/popoveranchor/main.tsx scripts/popover-anchor-drive.ts ND_POPOVER_ANCHOR_OK popoveranchor
