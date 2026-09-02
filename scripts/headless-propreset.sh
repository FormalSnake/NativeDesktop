#!/usr/bin/env bash
# scripts/headless-propreset.sh: the Linux half of the dropped-prop gate. Runs
# examples/propreset under weston-headless and drives it with
# scripts/propreset-drive.ts, the same script the AppKit leg runs.
#
# What it proves on GTK: a prop that leaves the app's JSX reaches the host as
# null and the generated appliers substitute the schema default, so `enabled`
# goes back to sensitive, `checked` to off, `label`/`tooltip` to empty, and a
# dropped `style` takes the box's padding off. Every leg is backend-neutral,
# so nothing here is skipped under ND_BACKEND=gtk.
set -euo pipefail
cd "$(dirname "$0")/.."

exec ./scripts/headless-run.sh \
  examples/propreset/main.tsx scripts/propreset-drive.ts ND_PROP_RESET_OK propreset
