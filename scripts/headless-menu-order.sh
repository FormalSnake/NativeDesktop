#!/usr/bin/env bash
# scripts/headless-menu-order.sh: the Linux half of the menu-order gate. Runs
# examples/notes/menu-order-probe.tsx under weston-headless and drives it with
# scripts/menu-order-drive.ts, the same script scripts/mac/mac-menu-order.sh
# runs on AppKit. Everything it exercises is backend-neutral: the drive reads
# the native menu back through the menuModel RPC and compares it to React's
# order after a move, a middle remove and a middle insert.
set -euo pipefail
cd "$(dirname "$0")/.."

exec ./scripts/headless-run.sh \
  examples/notes/menu-order-probe.tsx scripts/menu-order-drive.ts ND_MENU_ORDER_OK menuorder
