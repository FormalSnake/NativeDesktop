#!/usr/bin/env bash
# Copies the canonical C ABI headers (include/nd.h, include/nd_plugin.h) into
# the published @nativedesktop/native package verbatim. App plugins compile
# against the packaged copies, so any drift means a stale struct layout across
# the dlopen boundary. CI cmp-checks the pair — run this after editing include/.
set -euo pipefail
cd "$(dirname "$0")/.."

cp include/nd.h include/nd_plugin.h packages/native/include/
echo "synced include/nd.h + include/nd_plugin.h -> packages/native/include/"
