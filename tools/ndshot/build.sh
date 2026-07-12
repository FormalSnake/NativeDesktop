#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

# Inside the repo's nix devshell, a stale SDKROOT/DEVELOPER_DIR breaks the
# system Swift toolchain -- unset both so `swift build` uses Xcode's own.
env -u SDKROOT -u DEVELOPER_DIR swift build -c release

bin=".build/release/ndshot"

# Ad-hoc sign with a stable identifier so TCC's Screen Recording grant is
# tied to a consistent identity across rebuilds. Note: an ad hoc signature is
# derived from the binary's content hash, so this only preserves the grant
# when the compiled bytes are unchanged (path staying the same is not
# enough) -- a rebuild that actually changes the binary counts as a new
# identity to TCC and needs re-granting. Run `ndshot doctor` after a rebuild
# to check.
codesign -f -s - -i com.nativedesktop.ndshot "$bin"

echo "ndshot built and ad-hoc signed: $bin"
