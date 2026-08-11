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

# The template ships registry versions (^0.1.0) so a published copy installs
# from npm. A scaffold made from a checkout should exercise the checkout, not
# the registry: rewrite each @nativedesktop-family dep to a file: path relative
# to the destination.
DEST_ABS="$(cd "$DEST" && pwd)"
REL_PACKAGES="$(realpath --relative-to="$DEST_ABS" "$REPO_ROOT/packages")"
sed -i \
  -e "s#\"@nativedesktop/react\": \"[^\"]*\"#\"@nativedesktop/react\": \"file:${REL_PACKAGES}/react\"#" \
  -e "s#\"@nativedesktop/native\": \"[^\"]*\"#\"@nativedesktop/native\": \"file:${REL_PACKAGES}/native\"#" \
  -e "s#\"@nativedesktop/cli\": \"[^\"]*\"#\"@nativedesktop/cli\": \"file:${REL_PACKAGES}/nd\"#" \
  -e "s#\"babel-plugin-nativedesktop\": \"[^\"]*\"#\"babel-plugin-nativedesktop\": \"file:${REL_PACKAGES}/babel-plugin-nativedesktop\"#" \
  "$DEST/package.json"

# packages/nd itself depends on "@nativedesktop/host": "workspace:*", which
# only resolves inside this monorepo's workspace. Reached via the file: edge
# above from a destination outside it, that transitive spec can't resolve, so
# pin it the same way with an override.
DEST_PKG="$DEST/package.json" REL_PACKAGES="$REL_PACKAGES" bun -e '
const path = process.env.DEST_PKG;
const relPackages = process.env.REL_PACKAGES;
const pkg = await Bun.file(path).json();
pkg.overrides = { ...(pkg.overrides ?? {}), "@nativedesktop/host": `file:${relPackages}/host` };
await Bun.write(path, JSON.stringify(pkg, null, 2) + "\n");
'

echo "Scaffolded $NAME at $DEST"
echo "Next: cd $DEST && bun install && bun run dev"
