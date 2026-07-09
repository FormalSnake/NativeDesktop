#!/usr/bin/env bash
# Regenerates vendor/gobject-bindings from the pinned zig-gobject commit.
# Run inside the devshell (needs zig 0.16.0, xsltproc, and the GTK4 GIR files).
set -euo pipefail
cd "$(dirname "$0")/.."

PIN=97caf8bfb4386409aab1160f7ec05c32ee6d5d7d # feat: update to 0.16.0 (2026-04-20)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

git clone https://github.com/ianprime0509/zig-gobject "$WORK/zig-gobject"
git -C "$WORK/zig-gobject" checkout "$PIN"

FLAGS=()
while read -r d; do FLAGS+=("-Dgir-files-path=$d"); done < <(
  echo "$XDG_DATA_DIRS" | tr : '\n' | sort -u |
    while read -r p; do [ -d "$p/gir-1.0" ] && echo "$p/gir-1.0"; done
)

(cd "$WORK/zig-gobject" && zig build codegen -Dmodules=Gtk-4.0 "${FLAGS[@]}")

rm -rf vendor/gobject-bindings
cp -r "$WORK/zig-gobject/zig-out/bindings" vendor/gobject-bindings
echo "regenerated vendor/gobject-bindings from zig-gobject@$PIN"
