#!/usr/bin/env bash
# One Linux gate for the browser/automation wave. Run it from inside the flake
# dev shell (`nix develop -c ./scripts/browser-gate.sh`) — it never re-enters
# nix itself, so the environment it checks is the one it was given.
#
# Stages, in order of how fast they fail:
#   1. codegen freshness      — schema/*.json -> generated files, byte-stable
#   2. zig build              — the GTK host links
#   3. zig build test         — the unit-test binaries
#   4. headless-smoke         — a window maps under weston
#   5. headless-webview       — the webview surface + the page vocabulary
#   6. sourcetree drive       — row actions, iconData, the a11y probe
#   7. the app's own drives   — through the APP's own harness; a missing one
#                               SKIPs, a failing one reports unless
#                               ND_BROWSER_APP_STRICT=1
#
# Marker: ND_AUTO2_OK.
set -uo pipefail
cd "$(dirname "$0")/.."

APP_DIR="${ND_BROWSER_APP_DIR:-$HOME/Developer/nativebrowser}"
FAILED=()
APP_FAILED=()
SKIPPED=()

step() {
  local name="$1"; shift
  printf '\n=== %s ===\n' "$name"
  if "$@"; then
    echo "ND_GATE_STEP_OK $name"
  else
    echo "ND_GATE_STEP_FAIL $name"
    FAILED+=("$name")
  fi
}

# 1. Codegen freshness. Generation must be a no-op against a committed tree and
#    byte-stable across two runs — a generator whose output depends on its own
#    previous output silently drifts.
check_codegen() {
  bun tools/codegen.ts >/dev/null || return 1
  local after_first
  after_first=$(git status --porcelain src/generated packages/react/src/generated swift/Sources/NDGen)
  bun tools/codegen.ts >/dev/null || return 1
  local after_second
  after_second=$(git status --porcelain src/generated packages/react/src/generated swift/Sources/NDGen)
  if [ -n "$after_first" ]; then
    echo "generated files are stale — commit the output of \`bun tools/codegen.ts\`:"
    echo "$after_first"
    return 1
  fi
  if [ "$after_first" != "$after_second" ]; then
    echo "codegen is not byte-stable across two runs"
    return 1
  fi
}

check_build() { zig build; }

# `zig build test` prints a spurious "failed command ... --listen=-" for the
# GTK-linked binaries on a cold cache and passes on the rerun, so trust the
# second exit code (documented in SESSION-LEDGER.md).
check_tests() {
  local out
  out=$(zig build test --summary all 2>&1) || out=$(zig build test --summary all 2>&1) || {
    echo "$out" | tail -20
    return 1
  }
  echo "$out" | grep -oE "run test [0-9]+ pass" |
    awk '{s+=$3; n++} END {printf "ND_GATE_TESTS %d binaries, %d tests passed\n", n, s}'
}

check_smoke()   { ./scripts/headless-smoke.sh; }
check_webview() { ./scripts/headless-webview.sh; }

# The sourcetree drive spawns its own host (launchApp), so it needs a
# compositor but not the ND_AUTOMATION_SOCKET harness headless-run.sh sets up.
check_sourcetree() {
  export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
  export WAYLAND_DISPLAY=nd-headless-sourcetree
  export GSK_RENDERER=cairo GDK_BACKEND=wayland
  export ND_APP_ID=dev.nativedesktop.headless-sourcetree
  export ND_SHOT_DIR="${ND_SHOT_DIR:-$XDG_RUNTIME_DIR}"
  # The drive's own 3s waits are tuned for an idle machine; this runs right
  # after a full build, and the box may be shared.
  export ND_DRIVE_TIMEOUT_MS="${ND_DRIVE_TIMEOUT_MS:-15000}"
  . "$(dirname "$0")/headless-fonts.sh"
  weston --backend=headless --socket="$WAYLAND_DISPLAY" --idle-time=0 >"$XDG_RUNTIME_DIR/weston-sourcetree.log" 2>&1 &
  # Not a RETURN trap: it fires again when the calling `step` returns, by which
  # point the local it names is gone and `set -u` aborts the whole gate.
  ST_WESTON_PID=$!
  for _ in $(seq 1 50); do
    [ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ] && break
    sleep 0.1
  done
  local rc=0
  bun scripts/sourcetree-drive.ts gtk || rc=$?
  kill "$ST_WESTON_PID" 2>/dev/null
  return "$rc"
}

step "codegen freshness" check_codegen
step "zig build" check_build
step "zig build test" check_tests
step "headless smoke" check_smoke
step "headless webview" check_webview
step "sourcetree drive" check_sourcetree

# 7. The app's own drives, if the app repo is checked out beside us. Each runs
#    through the APP's harness (its scripts/headless*.sh), never re-invented
#    here. A missing drive is a SKIP: the app is written by a different agent on
#    a different schedule, and this gate certifies the FRAMEWORK. A drive that
#    exists and fails is reported but does not turn the gate red either, unless
#    ND_BROWSER_APP_STRICT=1 — set that once the app is done.
app_step() {
  local name="$1"; shift
  printf '\n=== app %s ===\n' "$name"
  if (cd "$APP_DIR" && "$@"); then
    echo "ND_GATE_STEP_OK app $name"
  elif [ "${ND_BROWSER_APP_STRICT:-0}" = "1" ]; then
    echo "ND_GATE_STEP_FAIL app $name"
    FAILED+=("app $name")
  else
    echo "ND_GATE_APP_FAIL app $name (not gating: set ND_BROWSER_APP_STRICT=1 to make it)"
    APP_FAILED+=("app $name")
  fi
}

if [ ! -d "$APP_DIR" ]; then
  SKIPPED+=("app drives: no app repo at $APP_DIR (set ND_BROWSER_APP_DIR)")
else
  for drive in smoke browser extensions; do
    wrapper="$APP_DIR/scripts/headless-$drive.sh"
    raw="$APP_DIR/scripts/$drive-drive.ts"
    if [ -f "$wrapper" ]; then
      app_step "$drive" bash "$wrapper"
    elif [ -f "$raw" ] && [ -f "$APP_DIR/scripts/headless.sh" ]; then
      # The app's headless.sh runs any command under its own compositor.
      app_step "$drive" bash "$APP_DIR/scripts/headless.sh" bun "scripts/$drive-drive.ts"
    else
      SKIPPED+=("app drive $drive: neither scripts/headless-$drive.sh nor scripts/$drive-drive.ts in $APP_DIR")
    fi
  done
fi

printf '\n=== summary ===\n'
for s in "${SKIPPED[@]:-}"; do [ -n "$s" ] && echo "ND_GATE_SKIP $s"; done
for a in "${APP_FAILED[@]:-}"; do [ -n "$a" ] && echo "ND_GATE_APP_FAIL $a"; done
if [ "${#FAILED[@]}" -gt 0 ]; then
  for f in "${FAILED[@]}"; do echo "ND_GATE_FAIL $f"; done
  echo "browser gate: ${#FAILED[@]} step(s) failed"
  exit 1
fi
echo "ND_AUTO2_OK browser gate green (${#SKIPPED[@]} skipped, ${#APP_FAILED[@]} app drive(s) red but not gating)"
