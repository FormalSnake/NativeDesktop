#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
REPO_ROOT="$(pwd)"

DEST="${1:?usage: new-app.sh <dest-dir>}"
[ -e "$DEST" ] && { echo "refusing: $DEST exists"; exit 1; }

cp -r template "$DEST"
mkdir -p "$DEST/docs/agents"
cp docs/agents/*.md "$DEST/docs/agents/"

NAME=$(basename "$DEST")
sed -i "s/nativedesktop-app/$NAME/" "$DEST/package.json"

# Rewrite every @nativedesktop-family file: path (react, host, nd) to point back
# at this checkout's packages/, relative to the destination (the template's own
# file:../packages/* only resolves when DEST is a sibling of template/ inside the repo).
DEST_ABS="$(cd "$DEST" && pwd)"
REL_PACKAGES="$(realpath --relative-to="$DEST_ABS" "$REPO_ROOT/packages")"
sed -i "s#file:../packages/#file:${REL_PACKAGES}/#g" "$DEST/package.json"

echo "Scaffolded $NAME at $DEST"
echo "Next: cd $DEST && bun install && bun run dev"
