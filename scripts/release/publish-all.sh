#!/usr/bin/env bash
# Publishes every public package in dependency order. `bun install` must have
# run in this checkout first: bun pm pack resolves workspace:* from the
# lockfile and crashes without it. DRY_RUN=1 switches to a --dry-run publish.
set -euo pipefail
cd "$(dirname "$0")/../.."

# bun pm pack rewrites workspace:* from the lockfile; npm publish then handles
# the OIDC token exchange and provenance, which bun publish cannot. Auth is
# trusted publishing only -- the release workflow deliberately leaves no token
# for npm to fall back on. Needs npm 11.5.1+; 10.x has no OIDC code path.
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
    # --loglevel verbose (not silly, which dumps request headers) so the OIDC
    # handshake shows up in the log. The exchange runs before the --dry-run
    # short circuit, so a dry run proves the trusted-publisher config too.
    log="$(mktemp)"
    npm publish "$tarball" "${flags[@]}" --loglevel verbose 2>&1 | tee "$log"
    grep -q 'oidc Successfully retrieved and set token' "$log" || {
      echo "FAIL: $dir did not authenticate through trusted publishing."
      echo "      Check that npmjs.com lists this repo and release.yml as the"
      echo "      trusted publisher for it, with 'npm publish' among the allowed actions."
      exit 1
    }
    rm -f "$log" "$tarball"
  )
done
