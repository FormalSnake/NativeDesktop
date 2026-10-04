#!/usr/bin/env bash
# The AppKit host turns on a Chromium feature by name (nd_cef.c, the floating
# video's title). A name Chromium no longer has is ignored without a word, so
# this checks the CEF framework still carries it. Marker: ND_PIP_FEATURE_OK.
set -euo pipefail
CEF_ROOT="${ND_CEF_ROOT:-$HOME/.cache/nativedesktop/cef/${ND_CEF_VERSION:-151.3.23-macosarm64}}"
BIN="$CEF_ROOT/Release/Chromium Embedded Framework.framework/Chromium Embedded Framework"
[ -f "$BIN" ] || BIN="$CEF_ROOT/Chromium Embedded Framework.framework/Chromium Embedded Framework"
NAME=VideoPipForceTrustedForMediaPlaybackForTesting
if LC_ALL=C grep -aqF "$NAME" "$BIN"; then
  echo "ND_PIP_FEATURE_OK $NAME is in $(basename "$CEF_ROOT")"
else
  echo "ND_PIP_FEATURE_FAIL $NAME is gone from $(basename "$CEF_ROOT"): the floating video keeps Chromium's title; see nd_cef.c"
  exit 1
fi
