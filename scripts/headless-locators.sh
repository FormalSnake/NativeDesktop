#!/usr/bin/env bash
# scripts/headless-locators.sh — the Linux half of the locator gate. Runs
# examples/locators under weston-headless and drives it with
# scripts/locator-drive.ts, the same script scripts/mac/mac-gestures.sh runs
# on AppKit. Everything it exercises is backend-neutral except the chord half
# of leg 1 (press/keyboard.type ride the `keys` RPC, -32003 on GTK), which the
# drive skips with a printed SKIP line when headless-run.sh hands it
# ND_BACKEND=gtk.
#
# Covered here: focus + the a11y focused probe, fill through setValue,
# scrollIntoView on a clipped row, node-sized snapshotNode, setWindowFrame,
# check/uncheck, selectOption by label.
set -euo pipefail
cd "$(dirname "$0")/.."

exec ./scripts/headless-run.sh \
  examples/locators/main.tsx scripts/locator-drive.ts ND_LOCATOR_OK locators
