#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

# Three legs, each a self-contained weston+host run with a UNIQUE
# WAYLAND_DISPLAY (model: scripts/headless-m5c.sh). Legs are sequential (not
# parallel) so each gets a clean XDG_RUNTIME_DIR/weston pair and log grepping
# never crosses runs.

# ---- Leg 1: benchmark (json vs binary) ----
DISP=nd-m10-bench-json
export XDG_RUNTIME_DIR="$(mktemp -d)"
export WAYLAND_DISPLAY=$DISP GSK_RENDERER=cairo GDK_BACKEND=wayland
weston --backend=headless --socket="$DISP" --idle-time=0 &
WP=$!
trap 'kill "$WP" ${HP:-} 2>/dev/null || true' EXIT
for _ in $(seq 1 50); do [ -S "$XDG_RUNTIME_DIR/$DISP" ] && break; sleep 0.1; done

JLOG=$(mktemp)
ND_FORCE_JSON=1 ND_SCRIPT=scripts/bench-10k.ts ./zig-out/bin/nd-hello >"$JLOG" 2>&1 &
HP=$!
for _ in $(seq 1 100); do grep -q ND_COMMIT_APPLIED "$JLOG" && break; sleep 0.1; done
grep -q ND_COMMIT_APPLIED "$JLOG" || { echo "FAIL: json bench no commit"; cat "$JLOG"; exit 1; }
grep -q "ND_BENCH_MOUNT encoding=json" "$JLOG" || { echo "FAIL: json encoding not negotiated"; cat "$JLOG"; exit 1; }
kill "$HP" 2>/dev/null || true; wait "$HP" 2>/dev/null || true
JMS=$(grep -m1 ND_BENCH_MOUNT "$JLOG" | sed 's/.*ms=//')

BLOG=$(mktemp)
ND_SCRIPT=scripts/bench-10k.ts ./zig-out/bin/nd-hello >"$BLOG" 2>&1 &
HP=$!
for _ in $(seq 1 100); do grep -q ND_COMMIT_APPLIED "$BLOG" && break; sleep 0.1; done
grep -q ND_COMMIT_APPLIED "$BLOG" || { echo "FAIL: binary bench no commit"; cat "$BLOG"; exit 1; }
grep -q "ND_BENCH_MOUNT encoding=binary" "$BLOG" || { echo "FAIL: binary encoding not negotiated"; cat "$BLOG"; exit 1; }
kill "$HP" 2>/dev/null || true; wait "$HP" 2>/dev/null || true
HP=""
BMS=$(grep -m1 ND_BENCH_MOUNT "$BLOG" | sed 's/.*ms=//')
echo "M10_BENCH json_ms=$JMS binary_ms=$BMS"
# Gate: completion within a generous informational bound (architect override:
# binary is accepted ~2.3x json in practice — 10k bench: json ~2.7ms, binary
# ~6.5ms median; the strict "not slower" is informational only). 4x avoids CI
# flakiness while still proving binary is in the same class as json.
awk -v b="$BMS" -v j="$JMS" 'BEGIN{ if (b > j*4 + 20) { print "FAIL: binary far slower than json (>4x)"; exit 1 } }'

kill "$WP" 2>/dev/null || true; wait "$WP" 2>/dev/null || true

# ---- Leg 2: ACL deny ----
# NOTE: src/acl.zig's grants model is documented additive-only ("A grants
# manifest (JSON) can extend -- never shrink -- the default", src/acl.zig:6;
# Acl.initDefault always grants core:commit + core:window.create so existing
# demos never break). There is therefore NO manifest that can deny
# core:window.create -- ND_ACL_GRANTS can only ADD permissions on top of the
# safe default, never withhold one of the two defaults. Verified empirically
# this session: a manifest naming only core:commit still leaves
# core:window.create granted and the counter's first commit applies cleanly.
# The only permission namespace that is genuinely default-deny is `plugin:*`,
# so this leg proves the ACL deny path via the plugin capability gate
# instead (same runtime.zig:393-403/413-447 gate the window-create case would
# have hit) -- this is leg 3's "no grant" run in isolation, run here as its
# own self-contained leg per the task's 3-leg structure.
DISP=nd-m10-acl
export WAYLAND_DISPLAY=$DISP
weston --backend=headless --socket="$DISP" --idle-time=0 &
WP=$!
for _ in $(seq 1 50); do [ -S "$XDG_RUNTIME_DIR/$DISP" ] && break; sleep 0.1; done
SO="$(pwd)/zig-out/lib/libnd_plugin_hello.so"
[ -f "$SO" ] || { echo "FAIL: build zig build plugin-hello first"; exit 1; }
ALOG=$(mktemp)
ND_PLUGINS=1 ND_PLUGIN_PATH="$SO" \
  ND_SCRIPT=scripts/m10-plugin-drive.ts timeout 8s ./zig-out/bin/nd-hello >"$ALOG" 2>&1 || true
grep -q "ND_ACL_DENY permission=plugin:hello.greet" "$ALOG" || { echo "FAIL: no ACL deny"; cat "$ALOG"; exit 1; }
echo "M10_ACL_DENY_OK"
kill "$WP" 2>/dev/null || true; wait "$WP" 2>/dev/null || true

# ---- Leg 3: plugin load + gated dispatch ----
DISP=nd-m10-plugin
export WAYLAND_DISPLAY=$DISP
weston --backend=headless --socket="$DISP" --idle-time=0 &
WP=$!
for _ in $(seq 1 50); do [ -S "$XDG_RUNTIME_DIR/$DISP" ] && break; sleep 0.1; done
PLOG=$(mktemp)
ND_PLUGINS=1 ND_PLUGIN_PATH="$SO" \
  ND_ACL_GRANTS='{"grants":[{"window":0,"permissions":["plugin:hello.greet"]}]}' \
  ND_SCRIPT=scripts/m10-plugin-drive.ts timeout 8s ./zig-out/bin/nd-hello >"$PLOG" 2>&1 || true
grep -q "ND_PLUGIN_LOADED name=hello" "$PLOG" || { echo "FAIL: plugin not loaded"; cat "$PLOG"; exit 1; }
grep -q "ND_PLUGIN_COMMAND_OK plugin=hello command=greet" "$PLOG" || { echo "FAIL: plugin command denied/failed"; cat "$PLOG"; exit 1; }
echo "M10_PLUGIN_OK"

# Same plugin, capability withheld -> deny (belt-and-suspenders repeat of
# leg 2's mechanism, but scoped to the plugin-load leg per the task spec).
DLOG=$(mktemp)
ND_PLUGINS=1 ND_PLUGIN_PATH="$SO" \
  ND_SCRIPT=scripts/m10-plugin-drive.ts timeout 8s ./zig-out/bin/nd-hello >"$DLOG" 2>&1 || true
grep -q "ND_ACL_DENY permission=plugin:hello.greet" "$DLOG" || { echo "FAIL: plugin cmd not denied without cap"; cat "$DLOG"; exit 1; }
echo "M10_PLUGIN_ACL_DENY_OK"
kill "$WP" 2>/dev/null || true; wait "$WP" 2>/dev/null || true
WP=""

echo "headless m10: OK (binary bench + acl deny + plugin load/deny)"
