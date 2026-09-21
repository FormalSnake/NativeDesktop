#!/usr/bin/env bash
# scripts/headless-headerfield.sh: the Linux half of the header-bar field gate.
# Runs examples/headerfield under weston-headless and drives it with
# scripts/headerfield-drive.ts, the same script the AppKit leg runs.
#
# What it proves on GTK: an AdwHeaderBar's title widget really takes the run
# between the start and end packs at three window widths (it used to stop at
# twice the distance to the nearer pack, because AdwHeaderBar centres its
# title), the leading icon inside the field delivers its event, a popover that
# named the icon slot opens on the icon, and setValue still emits `changed`
# through the GtkEntry the leading icon turns the field into.
set -euo pipefail
cd "$(dirname "$0")/.."

exec ./scripts/headless-run.sh \
  examples/headerfield/main.tsx scripts/headerfield-drive.ts ND_HEADERFIELD_OK headerfield
