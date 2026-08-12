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
    tarball="$(bun pm pack --quiet)"
    npm publish "$tarball" "${flags[@]}"
    rm -f "$tarball"
  )
done
