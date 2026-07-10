#!/usr/bin/env bash
set -euo pipefail
ssh macbook 'cd ~/nd && export PATH="/etc/profiles/per-user/kyandesutter/bin:$PATH" \
  && zig build libnd -Dbackend=abi 2>&1 | tail -5 \
  && ls -la zig-out/lib/libnd.a \
  && nm zig-out/lib/libnd.a 2>/dev/null | grep -E "nd_init|nd_register_backend|nd_start_runtime" | head'
echo "MAC_LIBND_OK"
