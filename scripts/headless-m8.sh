#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
export WAYLAND_DISPLAY=nd-headless-m8
export GSK_RENDERER=cairo GDK_BACKEND=wayland
export NATIVE_AUTOMATION=1 ND_DEV=1

# HMR leg edits a COPY, never the repo (owner mandate). The copy is created
# UNDER the repo tree (not /tmp): verified empirically this session that
# `bun --hot`'s file watcher does not pick up edits to a script living
# outside the host process's cwd tree (the host always runs from the repo
# root), so a /tmp copy silently never re-evaluates. `.m8-drive-*` is
# untracked and removed by the EXIT trap; never staged/committed.
APPDIR=$(mktemp -d -p "$(pwd)" .m8-drive-XXXXXX)

# Fixture note (M8-D2 fallback, recorded honestly): examples/counter's App
# uses `useState`. Verified empirically this session: `bun --hot` gives every
# module in the re-evaluated graph (including `react` and `react-reconciler`,
# not just the edited file) a FRESH module instance on each edit, while the
# surviving globalThis-held HMR state (packages/react/src/hmr.ts) still
# references the OLD reconciler/root. Re-rendering the OLD reconciler against
# a NEW `<App/>` whose `useState` resolves against the NEW react module's
# internals throws "Invalid hook call" (reproduced directly against
# examples/counter and template/'s fixtures, both hook-based) -- the child
# then reports runtimeError and exits, which is exactly the disconnect path
# the HMR leg must NOT hit. This is the M8-D2 "manual family registration
# proves too fragile" case, so this script uses the documented bounded
# fallback instead: a globalThis-backed store (no hooks at all), which
# sidesteps the cross-module-instance dispatcher mismatch entirely. The
# fixture below is intentionally minimal and lives only in this script (no
# repo file is added or edited) -- see the M8 plan's Task 8 fallback note.
cat >"$APPDIR/main.tsx" <<'EOF'
import { render } from "@nativedesktop/react";

declare global {
  var __nd_clicks: number | undefined;
}
globalThis.__nd_clicks = globalThis.__nd_clicks ?? 0;

function App(): React.ReactNode {
  const clicks = globalThis.__nd_clicks!;
  return (
    <window title="NativeDesktop M8 HMR fixture" defaultWidth={480} defaultHeight={320}>
      <box orientation="vertical" spacing={8}>
        <label testID="clicks-label" text={`Clicks: ${clicks}`} />
        <button
          testID="increment-button"
          label="Increment"
          onClick={() => {
            globalThis.__nd_clicks!++;
            render(<App />);
          }}
        />
      </box>
    </window>
  );
}

await render(<App />);
EOF
cat >"$APPDIR/package.json" <<'EOF'
{
  "name": "m8-hmr-fixture",
  "private": true,
  "type": "module",
  "dependencies": { "@nativedesktop/react": "workspace:*", "react": "19.2.7" }
}
EOF
cat >"$APPDIR/tsconfig.json" <<'EOF'
{ "extends": "../../tsconfig.base.json" }
EOF
# Reuse the workspace's already-resolved node_modules (examples/counter's
# symlinks are anchored relative to the repo, so they must be referenced,
# not copied -- copying `node_modules` breaks its relative symlinks).
ln -s "$(pwd)/examples/counter/node_modules" "$APPDIR/node_modules"
export ND_SCRIPT="$APPDIR/main.tsx"

weston --backend=headless --socket="$WAYLAND_DISPLAY" --idle-time=0 &
WESTON_PID=$!
trap 'kill "$WESTON_PID" "$HOST_PID" 2>/dev/null || true; rm -rf "$APPDIR"' EXIT
for _ in $(seq 1 50); do [ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ] && break; sleep 0.1; done

LOG=$(mktemp)
./zig-out/bin/nd-hello >"$LOG" 2>&1 &
HOST_PID=$!
for _ in $(seq 1 80); do grep -q ND_AUTOMATION_LISTENING "$LOG" && grep -q ND_COMMIT_APPLIED "$LOG" && break; sleep 0.1; done
grep -q ND_AUTOMATION_LISTENING "$LOG" || { echo "FAIL: no automation listener"; cat "$LOG"; exit 1; }
grep -q ND_COMMIT_APPLIED "$LOG" || { echo "FAIL: no commit applied"; cat "$LOG"; exit 1; }
SOCK=$(grep -m1 ND_AUTOMATION_LISTENING "$LOG" | sed 's/.*path=//')
export ND_AUTOMATION_SOCKET="$SOCK"

# ---- HMR leg: click to Clicks: 2, edit the COPY, assert label changes + count preserved + no ND_CHILD_EXITED ----
bun scripts/m8-drive.ts --hmr-check >"$XDG_RUNTIME_DIR/hmr1.log" 2>&1 || { echo "FAIL hmr precheck"; cat "$XDG_RUNTIME_DIR/hmr1.log" "$LOG"; exit 1; }
cat "$XDG_RUNTIME_DIR/hmr1.log"
grep -q M8_HMR_PRECHECK_OK "$XDG_RUNTIME_DIR/hmr1.log" || { echo "FAIL: precheck marker"; exit 1; }

EXITED_BEFORE=$(grep -c ND_CHILD_EXITED "$LOG" || true)
# State-preserving edit: change a label string only (NOT the counter).
sed -i 's/Increment/Increment!/' "$APPDIR/main.tsx"
sleep 2.5   # let --hot re-eval land

bun scripts/m8-drive.ts --hmr-verify >"$XDG_RUNTIME_DIR/hmr2.log" 2>&1 || { echo "FAIL hmr verify"; cat "$XDG_RUNTIME_DIR/hmr2.log" "$LOG"; exit 1; }
cat "$XDG_RUNTIME_DIR/hmr2.log"
grep -q M8_HMR_OK "$XDG_RUNTIME_DIR/hmr2.log" || { echo "FAIL: state not preserved across edit"; cat "$XDG_RUNTIME_DIR/hmr2.log"; exit 1; }
EXITED_AFTER=$(grep -c ND_CHILD_EXITED "$LOG" || true)
[ "$EXITED_BEFORE" = "$EXITED_AFTER" ] || { echo "FAIL: child exited during HMR (before=$EXITED_BEFORE after=$EXITED_AFTER)"; cat "$LOG"; exit 1; }

# ---- crash leg: kill the child, assert overlay present + (dev) restart recovers ----
BUN_PID=$(pgrep -P "$HOST_PID" -f bun | head -1)
kill -9 "$BUN_PID"
for _ in $(seq 1 50); do grep -q ND_OVERLAY_SHOWN "$LOG" && break; sleep 0.1; done
grep -q ND_OVERLAY_SHOWN "$LOG" || { echo "FAIL: no overlay after crash"; cat "$LOG"; exit 1; }
bun scripts/m8-drive.ts >"$XDG_RUNTIME_DIR/crash.log" 2>&1 || { echo "FAIL crash drive"; cat "$XDG_RUNTIME_DIR/crash.log" "$LOG"; exit 1; }
cat "$XDG_RUNTIME_DIR/crash.log"
grep -q M8_CRASH_OK "$XDG_RUNTIME_DIR/crash.log" || { echo "FAIL: crash recovery"; cat "$XDG_RUNTIME_DIR/crash.log"; exit 1; }

kill -TERM "$HOST_PID"; wait "$HOST_PID" 2>/dev/null || true
echo "headless m8: OK (HMR state-preserving edit + crash overlay + dev restart)"
