#!/usr/bin/env bash
# Publishes every public package in dependency order. `bun install` must have
# run in this checkout first: bun pm pack resolves workspace:* from the
# lockfile and crashes without it. DRY_RUN=1 switches to a --dry-run publish.
set -euo pipefail
cd "$(dirname "$0")/../.."

# bun pm pack rewrites workspace:* from the lockfile; npm publish handles
# auth (OIDC trusted publishing in CI when configured, .npmrc/NPM_CONFIG_TOKEN
# otherwise) and provenance, which bun publish cannot.
flags=(--access public)
if [ "${DRY_RUN:-0}" = "1" ]; then
  flags+=(--dry-run)
fi

# Order: platform binaries first, then host (their dependent), then the
# libraries, then the CLI (depends on host).
for dir in \
  packages/host-darwin-arm64 \
  packages/host-linux-x64 \
  packages/host \
  packages/react \
  packages/rpc \
  packages/test \
  packages/native \
  packages/data \
  packages/panes \
  packages/babel-plugin-nativedesktop \
  packages/nd; do
  echo "publishing $dir"
  (
    cd "$dir"
    # Take the last .tgz line rather than the whole of stdout: `bun pm pack
    # --quiet` also emits blank//informational lines, and the stray newline
    # ends up inside the path npm then fails to open (ENOENT on a filename
    # with a leading newline, which is how v0.1.0 and the first v0.1.1 tag
    # died on the very first package).
    tarball="$(bun pm pack --quiet | tr -d '\r' | awk '/\.tgz$/ { last = $0 } END { if (last != "") print last }')"
    [ -n "$tarball" ] || { echo "FAIL: bun pm pack printed no .tgz name in $dir"; exit 1; }
    [ -f "$tarball" ] || { echo "FAIL: bun pm pack reported '$tarball' in $dir but it is not a file"; exit 1; }
    npm publish "$tarball" "${flags[@]}"
    rm -f "$tarball"
  )
done
