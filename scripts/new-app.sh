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

# Rewrite the @nativedesktop/react file: path to point back at this checkout's
# packages/react, relative to the destination (the template's own file:../packages/react
# only resolves when DEST is a sibling of template/ inside the repo).
DEST_ABS="$(cd "$DEST" && pwd)"
REL_PATH="$(realpath --relative-to="$DEST_ABS" "$REPO_ROOT/packages/react")"
sed -i "s#file:../packages/react#file:${REL_PATH}#" "$DEST/package.json"

echo "Scaffolded $NAME at $DEST"
echo "Next: cd $DEST && bun install && ND_DEV=1 ND_SCRIPT=src/main.tsx <path-to-nd-host-binary>"
