# M10 — Plugins + hardening + binary fast path: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Milestones M1–M8 + M6 (macOS shell) are landed and green.** The core (`src/tree.zig`/`src/runtime.zig`/`src/automation.zig`/`src/protocol.zig`) is GTK-free behind the C vtable in `include/nd.h`; Linux routes through the `abi` seam (`src/abi.zig` + `src/abi_backend.zig`), the null backend serves conformance, and a Swift/AppKit embedder (`swift/`) links `libnd.a` on a real Mac over `ssh macbook`. This milestone adds four disjoint things on top of that frozen surface: a **binary command-buffer encoding** for `CommitBatch` (negotiated, JSON stays default), a **capability ACL** enforced at NDP dispatch, the **`nd_plugin_v1` native-plugin ABI** with a first-party demo plugin, and a **10k-node benchmark gate**. The wasmtime/Extism WASM tier is **explicitly descoped** to a design-doc section + ABI shaping (see Task 13 and "Owner-visible descopes" below).

**Goal:** Ship the binary NDP fast path (1:1 with the JSON op list, drop-in via handshake), a Tauri-v2-style capability ACL at NDP dispatch, the `nd_plugin_v1` native-plugin loading path with a demo plugin, and a headless 10k-node benchmark that gates the binary path — all while every existing JSON gate stays byte-for-byte green.

**Architecture:** Additive only. The binary encoder lives TS-side in `runtime/ndp.ts` (through its existing drain-driven outbox); the decoder lives Zig-side in a new `src/ndp_binary.zig` decoding into the existing `protocol.CommitBatch` so `tree.apply` is untouched. The ACL is a new `src/acl.zig` module consulted in `runtime.zig`'s frame loop before a command is applied. Plugins load via `dlopen` (Zig `std.DynLib`) behind an opt-in env flag, register through a versioned `nd_plugin_v1` C struct in a new `include/nd_plugin.h`, and their capability declarations are checked against the ACL. Grants and plugin paths enter the core through two **new C lifecycle functions** (`nd_set_acl`, `nd_load_plugin`) — the `nd_backend` vtable is not touched (no vtable churn), so `include/nd.h`'s contract stays a superset and every existing embedder keeps working.

**Tech Stack:** Zig 0.16.0 (pinned), Bun 1.3.13 (pinned), TypeScript, C ABI, SwiftPM (Mac wiring leg), Nix devshell, weston-headless CI.

---

## Global Constraints

Copied verbatim from the spec and the architect's binding decisions. Every task's requirements implicitly include this section.

- **JSON stays the default and is fully supported.** Conformance and every existing script (`headless-smoke/m2/m3/m4/m5b/m5c/m8`, `kill9-test`, `mac-m6`) must pass **unchanged** with JSON. Binary is negotiated only via the existing `HelloAck.encodings` handshake — **never a flag day** (spec D3, D4; binary spec §2).
- **The binary path implements `docs/superpowers/specs/2026-07-09-ndp-binary-encoding.md` 1:1 with the JSON op list.** Deviations are allowed only where the spec conflicts with reality; each must be documented in the code and in the plan. The `NDP_TRACE` tracer must decode either encoding to identical JSON text (spec §9).
- **`include/nd.h` is THE contract.** ACL grants and plugin loading enter via **new `nd_*` lifecycle functions**, never new `nd_backend` vtable fields. **ALL 18 `nd_backend` vtable fields must remain non-null** (the core calls them unconditionally per commit; a null fn ptr is SIGSEGV — M6b hard-won fact). The `@sizeOf(NdBackend) == 18 * @sizeOf(usize)` assert in `src/abi.zig` stays.
- **Default ACL policy must NOT break existing demos.** Core UI ops (the eight `CommitBatch` op kinds) are **granted by default**; privileged/plugin commands **default-deny**. Grants are per-window, namespaced (`core:commit`, `core:window.create`, `plugin:<name>.<perm>`).
- **Plugins are opt-in.** Native-plugin loading (`dlopen`) is gated behind an explicit host flag/env (`ND_PLUGINS=1` + `ND_PLUGIN_PATH=<abs>`); nothing loads a plugin unless the embedder asks.
- **wasmtime/Extism is descoped this milestone** — design-doc section + ABI shaped so the WASM tier is a second loader behind the same `nd_plugin_v1` surface (Task 13). This is an **owner-visible descope**, stated in the plan and `CLAUDE-activeContext.md`.
- **Bun lifecycle-script blocking (D12 hardening extra):** enforce `trustedDependencies` in `template/package.json` if trivial, else defer with a note (Task 12).
- **All existing gates stay green.** Full gate command (run before starting and after every wave):
```bash
nix develop -c bash -c 'bun tools/codegen.ts \
  && git diff --exit-code -- schema packages/react/src/generated src/generated docs/widgets.md docs/styling.md swift/Sources/NDGen/Widgets.swift \
  && zig build test \
  && zig build \
  && bun install --frozen-lockfile \
  && ./scripts/headless-smoke.sh \
  && ./scripts/headless-m2.sh \
  && ./scripts/kill9-test.sh \
  && ./scripts/headless-m3.sh \
  && ./scripts/headless-m4.sh \
  && ./scripts/headless-m5b.sh \
  && ./scripts/headless-m5c.sh \
  && ./scripts/headless-m8.sh'
```
- **The Mac shell (`swift/`) must keep building.** If `include/nd.h` changes shape (it does — two new functions + one new header), Task 11 adds the Swift-side no-op/wiring and a `ssh macbook` verification leg (mirror `scripts/mac/mac-m6.sh`).
- **Commit style:** short imperative lowercase subject with a conventional prefix matching recent history (`feat(binary):`, `feat(acl):`, `feat(plugin):`, `ci(m10):`). No co-author trailers; `git add` explicit paths per task — never `git add -A`; `node_modules`/caches/`CLAUDE*.md` never staged.

---

## Hard facts for implementers (read before touching any file)

These are load-bearing environment/codebase facts. An implementer subagent sees ONLY its own task text, so each fact it needs is repeated in-task — but read this list once to build a mental model.

**Zig 0.16 API drift (bake into every Zig task):**
- No `std.heap.GeneralPurposeAllocator`. The core uses `std.heap.page_allocator` (see `nd_init` in `src/abi.zig:75`); tests use `std.testing.allocator`.
- No `std.posix.getenv`/`unlink`. Sockets live in `std.Io.net`. `std.Io.Mutex` needs `.init` as its default (a bare `self.* = undefined` skips field defaults and bricks it — see `src/runtime.zig:21,54`).
- **`std.time.milliTimestamp` and `std.Thread.sleep` are GONE.** For timing use `std.Io` primitives; for the benchmark, prefer **poll-count/marker discipline** over wall-clock (architect decision — timings are informational markers, pass/fail is completion within a generous bound).
- **`@cImport` is gone** — C translation goes through the build system; but this milestone needs no C translation (plugins `dlopen` a `.so` and cast fn pointers).
- **Every test-bearing `.zig` file needs its OWN `addTest` root in `build.zig`.** Tests are NOT collected transitively through `@import` (this silently skipped `style.zig`'s tests until wired — M5c fact). Any new `src/*.zig` with `test {}` blocks MUST get a `test_step.dependOn(&b.addRunArtifact(<its test>).step)` entry.
- **Zig 0.16 module rule:** a module's `@import` cannot escape its root_source_file's directory. New core files stay flat under `src/` so they reach `protocol.zig`/`tree.zig` via same-directory relative imports (like `abi.zig` does). `src/abi.zig` is the one module root that transitively covers the whole GTK-free core (`abi -> {abi_backend, tree, runtime, automation, protocol}`), and `src/core/root.zig` re-exports it as the named module `"abi"` for `libnd.a`.

**NDP / protocol facts:**
- Outer frame is `u32 LE length ‖ payload`, shared by JSON and binary (binary spec §1.1). `src/protocol.zig`'s `encodeFrame` writes it; `runtime/ndp.ts`'s `send`/`onData` mirror it.
- The Zig reader loop is `src/runtime.zig:183-205` (`readerLoop`'s frame loop): it reads a frame, `protocol.peekType`s the JSON `type`, and routes `commitBatch`→`marshalCommit` / `ping`→pong / `runtimeError`→stash / else drop. **This is where binary-frame routing and ACL enforcement hook in.**
- The HelloAck is sent at `src/runtime.zig:179`: `self.writeFrame(protocol.HelloAck{ .ndpVersion = protocol.ndp_version, .encodings = &.{"json"} });` — **this `&.{"json"}` becomes `&.{ "binary", "json" }` when the host supports binary (host preference order, binary first).**
- The commit apply path is `applyOnUi` (`src/runtime.zig:318-330`): it `std.json.parseFromSlice`s the JSON `CommitBatch` and calls `self.tree.apply(parsed.value)`. The binary path must produce the **same `protocol.CommitBatch`** and call the **same `tree.apply`** — do not fork `tree.apply`.
- `protocol.CommitBatch` = `{ type, commitId: u64, generation: u32, ops: []Op }` (`src/protocol.zig:90-95`). `Op` (`src/protocol.zig:75-88`) is a permissive struct: `op: []const u8` discriminator + optional `id/widget/props/parent/child/text/before`. `props: ?std.json.Value`.
- **The 8 ops (JSON `op` strings), exhaustive, in `tree.apply`'s dispatch (`src/tree.zig:172-254`):** `create`, `append`, `insertBefore`, `remove`, `setText`, `update`, `hide`, `unhide`. The binary opcode table (spec §5.1) is `0x01..0x08` in that exact order.
- **`runtime/ndp.ts` Bun-socket fact (M5c, load-bearing):** `Bun.Socket.write()` does NOT buffer partial writes — big frames silently truncate. `runtime/ndp.ts` already has a drain-driven FIFO outbox (`outbox`/`outboxOffset`/`pump`, lines 43-134). **Binary frames MUST go through `send()` → the same outbox**, never a raw `socket.write`.
- **Node id packing (both sides must agree):** `packages/react/src/ids.ts:16` → `(generation << 24) | (seq & 0xffffff)`. Binary node IDs are `u32` LE, high 8 bits generation, low 24 bits seq; `0` = "no node" (used by `insertBefore.before = 0` ≡ JSON `null`). This matches `src/tree.zig`'s comment at line 56-63 (**generation is 8 bits, not 16** — `0xFF` is the reserved overlay generation, `OVERLAY_GENERATION`).
- **`widgetType` u16 enum (binary spec §7)** is assigned in schema declaration order, never renumbered/reused. Current `schema/widgets.json` order (value `0` reserved/invalid): `1 Window, 2 Box, 3 Label, 4 Button, 5 TextInput, 6 TextArea, 7 Checkbox, 8 Radio, 9 Select, 10 Slider, 11 ProgressBar, 12 Image, 13 ScrollView, 14 Separator, 15 Spinner, 16 TabView, 17 Grid, 18 ListView, 19 WebView`. Codegen (`tools/codegen.ts`) is the single source of truth — Task 5 emits this table for BOTH the TS encoder and the Zig decoder from the schema.
- **Value tags (spec §5.3):** `0x00 null` (0 bytes), `0x01 bool` (u8), `0x02 i64` (i64 LE), `0x03 f64` (f64 LE), `0x04 stringRef` (u32 index). No array/object tag exists (spec §5.3, §10 — none in landed props).
- **String table (spec §6):** at `stringTableOffset` (header offset 24, u32 LE, from payload start). `u32 count`, then per entry `u32 len` + `len` UTF-8 bytes (no NUL). Per-`CommitBatch` interning, dedup mandatory.
- **Header (spec §3.1), 28 bytes:** `[0]=magic 0x4E`, `[1]=version 0x01`, `[2..8]=6 reserved zero`, `[8..16]=commitId u64 LE`, `[16..20]=generation u32 LE`, `[20..24]=opCount u32 LE`, `[24..28]=stringTableOffset u32 LE`. Op stream begins at offset 28. The **§8 golden vector** is the byte-for-byte fixture both sides must reproduce (corrected `stringTableOffset = 83`, total payload 136).

**Scripts / CI facts:**
- Every headless script `cd`s to repo root, sets a **unique** `WAYLAND_DISPLAY` (weston socket name) to avoid CI collisions, exports `GSK_RENDERER=cairo GDK_BACKEND=wayland NATIVE_AUTOMATION=1`, polls up to N×0.1s for `ND_AUTOMATION_LISTENING`+`ND_COMMIT_APPLIED`, greps `ND_*` markers, and traps to kill weston+host on EXIT. `scripts/headless-m5c.sh` is the model.
- `ND_*` markers print to **stderr**; scripts capture `2>&1`. Env the host reads: `ND_SCRIPT` (child TS entry), `ND_SOCKET`, `NDP_TRACE=1`.
- CI is `.github/workflows/ci.yml` — a linear list of `nix develop -c <cmd>` steps. **`bun test runtime/` is NOT yet in CI** (the existing `runtime/ndp.test.ts` runs manually). Task 4 wires it in.
- Mac legs run over `ssh macbook 'bash -euo pipefail -s' <<'REMOTE'` heredocs (login shell is fish); `export PATH="/etc/profiles/per-user/kyandesutter/bin:$PATH"`; the Zig `.a` must be repacked with system `ar`/`libtool` before swiftc links (M6b recipe, already in `scripts/mac/mac-m6.sh`). No GitHub push access from this box — CI stretch jobs are `continue-on-error`.

---

## File structure (what each new/changed file owns)

| File | New/Modify | Owner task | Responsibility |
|---|---|---|---|
| `tools/codegen.ts` | Modify | T5 (sole owner) | Emit `widgetType` u16 table → TS (`packages/react/src/generated/widget-types.ts`) + Zig (`src/generated/widget_types.zig`) from schema order. |
| `packages/react/src/generated/widget-types.ts` | New (generated) | T5 | `WIDGET_TYPE: Record<string, number>` for the TS encoder. |
| `src/generated/widget_types.zig` | New (generated) | T5 | `widgetTypeOf(name) u16` + `widgetNameOf(u16)` for the Zig decoder/tracer. |
| `src/ndp_binary.zig` | New | T2 | Binary `CommitBatch` decoder + tracer-to-JSON; own `addTest` root. |
| `runtime/ndp-binary.ts` | New | T3 | Binary `CommitBatch` encoder (pure, returns `Uint8Array` payload). |
| `runtime/ndp.ts` | Modify | T3 (sole owner) | Wire binary encoding selection into `send`/`sendCommit` via the outbox. |
| `runtime/ndp-binary.test.ts` | New | T3 | TS golden-frame test (spec §8 vector). |
| `src/acl.zig` | New | T6 | Capability ACL model + parse grants JSON + `isAllowed`; own `addTest` root. |
| `include/nd.h` | Modify | T10 (sole owner) | Add `nd_set_acl` + `nd_load_plugin` prototypes; `#include "nd_plugin.h"`. |
| `include/nd_plugin.h` | New | T8 (drafts), T10 (finalizes include) | `nd_plugin_v1` struct + `nd_plugin_registry` host callbacks. |
| `src/abi.zig` | Modify | T7 (ACL), T10 (plugin fns) — serialize | Add `nd_set_acl`/`nd_load_plugin` exports + `NdContext.acl`/`plugins` fields. |
| `src/plugin.zig` | New | T9 | `dlopen` loader, `nd_plugin_v1` mirror struct, registry impl; own `addTest` root. |
| `src/runtime.zig` | Modify | T7 (sole owner) | Binary-frame routing + ACL enforcement in `readerLoop`. |
| `plugins/hello/` | New | T9 | First-party demo plugin (Zig shared lib) registering one command. |
| `swift/Sources/NDShell/main.swift` | Modify | T11 | Call new `nd_set_acl`/`nd_load_plugin` (no-op-safe) so the Mac shell keeps building. |
| `scripts/bench-10k.ts` | New | T4 | Bun driver that mounts a 10k-node tree (JSON + binary legs). |
| `scripts/headless-m10.sh` | New | T14 | Benchmark + ACL-deny + plugin-load integration legs. |
| `build.zig` | Modify | T2/T6/T9 add `addTest` roots + `nd-plugin-hello` artifact — serialize | Wire new test roots and the demo-plugin shared-lib build step. |
| `.github/workflows/ci.yml` | Modify | T14 | Add `bun test runtime/` + `headless-m10.sh` steps. |
| `docs/superpowers/specs/2026-07-11-wasm-plugin-tier.md` | New | T13 | WASM-tier deferral rationale + how it slots behind `nd_plugin_v1`. |
| `CLAUDE-activeContext.md` | Modify | T14 | M10 done + hard-won facts. |

---

## Wave structure (parallelism)

- **Wave A (foundation, serial-ish):** T5 (codegen widget-type table — sole owner of `tools/codegen.ts`). Everything downstream consumes the emitted tables, so T5 lands first.
- **Wave B (parallel, disjoint files):** T2 (Zig binary decoder — `src/ndp_binary.zig` + build.zig test root), T3 (TS binary encoder — `runtime/ndp-binary.ts` + `runtime/ndp.ts` + test), T6 (ACL model — `src/acl.zig` + build.zig test root), T8 (`include/nd_plugin.h` draft + demo-plugin source skeleton `plugins/hello/`). These four touch disjoint files (T2/T6 both add a `build.zig` test root — **serialize their build.zig edits**, see note).
- **Wave C (integration, serial where files overlap):** T7 (wire binary routing + ACL into `src/runtime.zig` — depends on T2, T6), T9 (plugin loader `src/plugin.zig` + demo-plugin build — depends on T8), T10 (`include/nd.h` + `src/abi.zig` exports — depends on T6, T9; sole owner of both files this wave), T4 (benchmark driver — depends on T3), T11 (Swift wiring — depends on T10).
- **Wave D (finish):** T12 (Bun trustedDependencies hardening), T13 (WASM deferral doc), T14 (headless-m10.sh + CI + activeContext).

**build.zig serialization note:** T2, T6, and T9 each add a `test_step.dependOn` line (and T9 adds an artifact). To avoid conflicting edits, apply them in task-number order; each task's build.zig step shows the exact insertion point and the exact surrounding lines so a merge is trivial.

---

## TASK 5 — Codegen: emit the `widgetType` u16 table (both sides)

**Wave A (foundation, sole owner of `tools/codegen.ts` for this milestone). Depends on: nothing.**

**Files:**
- Modify: `tools/codegen.ts` (add `genWidgetTypesTs` + `genWidgetTypesZig` + two `writeIfChanged` calls at the bottom, next to lines 1353-1358)
- Create (generated): `packages/react/src/generated/widget-types.ts`
- Create (generated): `src/generated/widget_types.zig`

**Interfaces:**
- Produces (TS): `export const WIDGET_TYPE: Record<string, number>` mapping widget `name` → 1-based schema-order index; `export const WIDGET_TYPE_RESERVED = 0`.
- Produces (Zig): `pub fn widgetTypeOf(name: []const u8) ?u16` and `pub fn widgetNameOf(v: u16) ?[]const u8` over the same table.
- Both are 1:1 with binary spec §7 (value `0` reserved; schema order = enum order, never renumbered).

- [ ] **Step 1: Write the failing test (TS side).** Add to a new file `packages/react/src/generated/widget-types.test.ts`:

```typescript
import { test, expect } from "bun:test";
import { WIDGET_TYPE, WIDGET_TYPE_RESERVED } from "./widget-types";

test("widget types are 1-based schema order, 0 reserved", () => {
  expect(WIDGET_TYPE_RESERVED).toBe(0);
  expect(WIDGET_TYPE.Window).toBe(1);
  expect(WIDGET_TYPE.Box).toBe(2);
  expect(WIDGET_TYPE.Label).toBe(3);
  expect(WIDGET_TYPE.Button).toBe(4);
  expect(WIDGET_TYPE.ListView).toBe(18);
  expect(WIDGET_TYPE.WebView).toBe(19);
});
```

- [ ] **Step 2: Run it to verify it fails.**

Run: `nix develop -c bun test packages/react/src/generated/widget-types.test.ts`
Expected: FAIL — `Cannot find module './widget-types'` (file not generated yet).

- [ ] **Step 3: Add the emitters to `tools/codegen.ts`.** Mirror the existing `genSchemaMeta`/`genZig` shape. Add these two functions (near the other `gen*` functions):

```typescript
const HEADER_TS = "// GENERATED by tools/codegen.ts — do not edit\n";
const HEADER_ZIG = "// GENERATED by tools/codegen.ts — do not edit\n";

// Binary NDP widgetType enum (ndp-binary spec §7): schema declaration order,
// 1-based, value 0 reserved/invalid, NEVER renumbered or reused.
function genWidgetTypesTs(s: Schema): string {
  let out = HEADER_TS + "export const WIDGET_TYPE_RESERVED = 0;\n";
  out += "export const WIDGET_TYPE: Record<string, number> = {\n";
  for (let i = 0; i < s.widgets.length; i++) {
    out += `  ${s.widgets[i]!.name}: ${i + 1},\n`;
  }
  out += "};\n";
  return out;
}

function genWidgetTypesZig(s: Schema): string {
  let out = HEADER_ZIG + "const std = @import(\"std\");\n\n";
  out += "const Entry = struct { name: []const u8, value: u16 };\n";
  out += "pub const widget_types = [_]Entry{\n";
  for (let i = 0; i < s.widgets.length; i++) {
    out += `    .{ .name = ${JSON.stringify(s.widgets[i]!.name)}, .value = ${i + 1} },\n`;
  }
  out += "};\n\n";
  out += "pub fn widgetTypeOf(name: []const u8) ?u16 {\n";
  out += "    for (widget_types) |e| if (std.mem.eql(u8, e.name, name)) return e.value;\n";
  out += "    return null;\n}\n\n";
  out += "pub fn widgetNameOf(v: u16) ?[]const u8 {\n";
  out += "    for (widget_types) |e| if (e.value == v) return e.name;\n";
  out += "    return null;\n}\n";
  return out;
}
```

- [ ] **Step 4: Register the two new outputs.** At the bottom of `tools/codegen.ts` (right after the existing `await writeIfChanged("swift/Sources/NDGen/Widgets.swift", genSwift(schema));` line), add:

```typescript
await writeIfChanged("packages/react/src/generated/widget-types.ts", genWidgetTypesTs(schema));
await writeIfChanged("src/generated/widget_types.zig", genWidgetTypesZig(schema));
```

- [ ] **Step 5: Regenerate and run the test.**

Run: `nix develop -c bash -c 'bun tools/codegen.ts && bun test packages/react/src/generated/widget-types.test.ts'`
Expected: both generated files written; the TS test PASSES.

- [ ] **Step 6: Verify existing generated bytes are unchanged (freshness gate).**

Run: `nix develop -c bash -c 'bun tools/codegen.ts && git diff --exit-code -- packages/react/src/generated/intrinsics.ts packages/react/src/generated/schema-meta.ts src/generated/widgets.zig docs/widgets.md docs/styling.md swift/Sources/NDGen/Widgets.swift'`
Expected: exit 0 — only the two NEW files are added; no existing generated file shifts.

- [ ] **Step 7: Commit.**

```bash
git add tools/codegen.ts packages/react/src/generated/widget-types.ts packages/react/src/generated/widget-types.test.ts src/generated/widget_types.zig
git commit -m "feat(codegen): emit widgetType u16 table for binary NDP (spec §7)"
```

---

## TASK 2 — Zig binary `CommitBatch` decoder + tracer

**Wave B (parallel; disjoint file `src/ndp_binary.zig`; adds ONE build.zig test root — apply build.zig edit before T6's). Depends on: T5 (`src/generated/widget_types.zig`).**

**Files:**
- Create: `src/ndp_binary.zig`
- Modify: `build.zig` (add a `binary_tests` `addTest` root — insertion point shown below)

**Interfaces:**
- Consumes: `protocol.CommitBatch`/`protocol.Op` (`src/protocol.zig`), `widget_types.widgetNameOf` (T5).
- Produces: `pub fn decodeCommitBatch(gpa, payload: []const u8) !Decoded` where `Decoded` owns an arena and exposes `.batch: protocol.CommitBatch` for `tree.apply`; `pub fn isBinaryPayload(payload) bool` (magic+version sniff); `pub fn traceToJson(gpa, payload) ![]u8` (tracer, §9).

**Why a separate file, not `protocol.zig`:** the binary spec §10 says `protocol.zig` is JSON-only; the decoder is a new sibling. It reaches `protocol.zig` and `generated/widget_types.zig` via same-directory / `generated/` relative imports (both are flat under `src/`).

- [ ] **Step 1: Write the failing test — the §8 golden vector round-trips.** Create `src/ndp_binary.zig` with ONLY the tests first (implementation stubs return errors), then fill. The test encodes the spec's §8 example bytes as a literal and asserts the decode:

```zig
const std = @import("std");
const protocol = @import("protocol.zig");
const widget_types = @import("generated/widget_types.zig");

// ndp-binary spec §8 golden vector: the counter demo's first commit.
// commitId=1, generation=0, ops=[create Window{title:Hi}, create Label{text:Clicks: 0},
// append(1,2), setText(2,"Clicks: 1")]. Bytes reproduced from §8.3/§8.4.
const golden_payload = [_]u8{
    // header (28 bytes)
    0x4e, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // magic,version,6x reserved
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // commitId=1
    0x00, 0x00, 0x00, 0x00, // generation=0
    0x04, 0x00, 0x00, 0x00, // opCount=4
    0x53, 0x00, 0x00, 0x00, // stringTableOffset=83 (§8.4 corrected)
    // op[0] create Window(1) props{title:"Hi"}  (19 bytes)
    0x01, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x04, 0x01, 0x00, 0x00, 0x00,
    // op[1] create Label(2) props{text:"Clicks: 0"}  (18 bytes)
    0x01, 0x02, 0x00, 0x00, 0x00, 0x03, 0x00, 0x01, 0x00,
    0x02, 0x00, 0x00, 0x00, 0x04, 0x03, 0x00, 0x00, 0x00,
    // op[2] append(1,2)  (9 bytes)
    0x02, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00,
    // op[3] setText(2, ref 4)  (9 bytes)
    0x05, 0x02, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00,
    // string table @83: count=5
    0x05, 0x00, 0x00, 0x00,
    0x05, 0x00, 0x00, 0x00, 0x74, 0x69, 0x74, 0x6c, 0x65, // "title"
    0x02, 0x00, 0x00, 0x00, 0x48, 0x69, // "Hi"
    0x04, 0x00, 0x00, 0x00, 0x74, 0x65, 0x78, 0x74, // "text"
    0x09, 0x00, 0x00, 0x00, 0x43, 0x6c, 0x69, 0x63, 0x6b, 0x73, 0x3a, 0x20, 0x30, // "Clicks: 0"
    0x09, 0x00, 0x00, 0x00, 0x43, 0x6c, 0x69, 0x63, 0x6b, 0x73, 0x3a, 0x20, 0x31, // "Clicks: 1"
};

test "golden vector decodes to the expected CommitBatch" {
    const gpa = std.testing.allocator;
    var decoded = try decodeCommitBatch(gpa, &golden_payload);
    defer decoded.deinit();
    const b = decoded.batch;
    try std.testing.expectEqual(@as(u64, 1), b.commitId);
    try std.testing.expectEqual(@as(u32, 0), b.generation);
    try std.testing.expectEqual(@as(usize, 4), b.ops.len);
    try std.testing.expectEqualStrings("create", b.ops[0].op);
    try std.testing.expectEqualStrings("Window", b.ops[0].widget.?);
    try std.testing.expectEqualStrings("Hi", b.ops[0].props.?.object.get("title").?.string);
    try std.testing.expectEqualStrings("Label", b.ops[1].widget.?);
    try std.testing.expectEqualStrings("append", b.ops[2].op);
    try std.testing.expectEqual(@as(u32, 1), b.ops[2].parent.?);
    try std.testing.expectEqual(@as(u32, 2), b.ops[2].child.?);
    try std.testing.expectEqualStrings("setText", b.ops[3].op);
    try std.testing.expectEqualStrings("Clicks: 1", b.ops[3].text.?);
}

test "isBinaryPayload sniffs magic+version" {
    try std.testing.expect(isBinaryPayload(&golden_payload));
    try std.testing.expect(!isBinaryPayload("{\"type\":\"commitBatch\"}"));
    try std.testing.expect(!isBinaryPayload(&[_]u8{ 0x4e, 0x02 })); // wrong version
}

test "traceToJson round-trips golden to greppable JSON" {
    const gpa = std.testing.allocator;
    const json = try traceToJson(gpa, &golden_payload);
    defer gpa.free(json);
    try std.testing.expect(std.mem.indexOf(u8, json, "\"commitId\":1") != null);
    try std.testing.expect(std.mem.indexOf(u8, json, "\"op\":\"create\"") != null);
    try std.testing.expect(std.mem.indexOf(u8, json, "\"widget\":\"Window\"") != null);
    try std.testing.expect(std.mem.indexOf(u8, json, "\"text\":\"Clicks: 1\"") != null);
}

test "rejects bad magic/version/opcode loudly" {
    const gpa = std.testing.allocator;
    var bad = golden_payload;
    bad[0] = 0x00; // wrong magic
    try std.testing.expectError(error.BadMagic, decodeCommitBatch(gpa, &bad));
}
```

- [ ] **Step 2: Run to verify it fails.**

Run: `nix develop -c zig test src/ndp_binary.zig` (quick standalone check; the wired root comes in Step 5)
Expected: compile error / FAIL — `decodeCommitBatch` undefined.

- [ ] **Step 3: Implement the decoder.** Add above the tests. All reads are explicit little-endian via `std.mem.readInt` (no memory-mapping — spec §3 "no alignment requirement"). Use an arena owned by `Decoded` so freeing is one call and the `[]Op`/strings/`std.json.Value` props stay alive for `tree.apply`:

```zig
pub const Decoded = struct {
    arena: *std.heap.ArenaAllocator,
    batch: protocol.CommitBatch,
    pub fn deinit(self: *Decoded) void {
        const gpa = self.arena.child_allocator;
        self.arena.deinit();
        gpa.destroy(self.arena);
    }
};

pub fn isBinaryPayload(payload: []const u8) bool {
    return payload.len >= 2 and payload[0] == 0x4e and payload[1] == 0x01;
}

const StringTable = struct {
    entries: [][]const u8,
    fn get(self: StringTable, idx: u32) ![]const u8 {
        if (idx >= self.entries.len) return error.BadStringRef;
        return self.entries[idx];
    }
};

fn readStringTable(a: std.mem.Allocator, payload: []const u8, off: u32) !StringTable {
    if (off + 4 > payload.len) return error.Truncated;
    var p: usize = off;
    const count = std.mem.readInt(u32, payload[p..][0..4], .little); p += 4;
    const entries = try a.alloc([]const u8, count);
    for (entries) |*e| {
        if (p + 4 > payload.len) return error.Truncated;
        const len = std.mem.readInt(u32, payload[p..][0..4], .little); p += 4;
        if (p + len > payload.len) return error.Truncated;
        e.* = try a.dupe(u8, payload[p .. p + len]); // owned by arena
        p += len;
    }
    return .{ .entries = entries };
}

fn decodeProps(a: std.mem.Allocator, payload: []const u8, p: *usize, prop_count: u16, st: StringTable) !std.json.Value {
    var obj = std.json.ObjectMap.init(a);
    var i: u16 = 0;
    while (i < prop_count) : (i += 1) {
        const key_ref = std.mem.readInt(u32, payload[p.*..][0..4], .little); p.* += 4;
        const key = try st.get(key_ref);
        const tag = payload[p.*]; p.* += 1;
        const v: std.json.Value = switch (tag) {
            0x00 => .null,
            0x01 => blk: { const b = payload[p.*] != 0; p.* += 1; break :blk .{ .bool = b }; },
            0x02 => blk: { const n = std.mem.readInt(i64, payload[p.*..][0..8], .little); p.* += 8; break :blk .{ .integer = n }; },
            0x03 => blk: { const bits = std.mem.readInt(u64, payload[p.*..][0..8], .little); p.* += 8; break :blk .{ .float = @bitCast(bits) }; },
            0x04 => blk: { const r = std.mem.readInt(u32, payload[p.*..][0..4], .little); p.* += 4; break :blk .{ .string = try st.get(r) }; },
            else => return error.BadValueTag,
        };
        try obj.put(key, v);
    }
    return .{ .object = obj };
}

pub fn decodeCommitBatch(gpa: std.mem.Allocator, payload: []const u8) !Decoded {
    if (payload.len < 28) return error.Truncated;
    if (payload[0] != 0x4e) return error.BadMagic;
    if (payload[1] != 0x01) return error.BadVersion;
    const arena = try gpa.create(std.heap.ArenaAllocator);
    arena.* = std.heap.ArenaAllocator.init(gpa);
    errdefer { arena.deinit(); gpa.destroy(arena); }
    const a = arena.allocator();

    const commit_id = std.mem.readInt(u64, payload[8..16], .little);
    const generation = std.mem.readInt(u32, payload[16..20], .little);
    const op_count = std.mem.readInt(u32, payload[20..24], .little);
    const st_off = std.mem.readInt(u32, payload[24..28], .little);
    const st = try readStringTable(a, payload, st_off);

    const ops = try a.alloc(protocol.Op, op_count);
    var p: usize = 28;
    for (ops) |*op| {
        const opcode = payload[p]; p += 1;
        op.* = protocol.Op{ .op = undefined };
        switch (opcode) {
            0x01 => { // create
                op.op = "create";
                op.id = std.mem.readInt(u32, payload[p..][0..4], .little); p += 4;
                const wt = std.mem.readInt(u16, payload[p..][0..2], .little); p += 2;
                op.widget = widget_types.widgetNameOf(wt) orelse return error.BadWidgetType;
                const pc = std.mem.readInt(u16, payload[p..][0..2], .little); p += 2;
                op.props = try decodeProps(a, payload, &p, pc, st);
            },
            0x02 => { op.op = "append"; op.parent = readId(payload, &p); op.child = readId(payload, &p); },
            0x03 => { op.op = "insertBefore"; op.parent = readId(payload, &p); op.child = readId(payload, &p);
                const before = readId(payload, &p); op.before = if (before == 0) null else before; },
            0x04 => { op.op = "remove"; op.id = readId(payload, &p); },
            0x05 => { op.op = "setText"; op.id = readId(payload, &p);
                const r = std.mem.readInt(u32, payload[p..][0..4], .little); p += 4; op.text = try st.get(r); },
            0x06 => { op.op = "update"; op.id = readId(payload, &p);
                const pc = std.mem.readInt(u16, payload[p..][0..2], .little); p += 2;
                op.props = try decodeProps(a, payload, &p, pc, st); },
            0x07 => { op.op = "hide"; op.id = readId(payload, &p); },
            0x08 => { op.op = "unhide"; op.id = readId(payload, &p); },
            else => return error.BadOpcode, // spec §5.1: reject, never skip (would desync)
        }
    }
    return .{ .arena = arena, .batch = .{ .commitId = commit_id, .generation = generation, .ops = ops } };
}

fn readId(payload: []const u8, p: *usize) u32 {
    const v = std.mem.readInt(u32, payload[p.*..][0..4], .little); p.* += 4;
    return v;
}
```

- [ ] **Step 4: Implement `traceToJson`.** Decode, then `std.json.Stringify.valueAlloc` the `CommitBatch` with the same options `protocol.encodeFrame` uses (so it's byte-identical to the JSON path's `commitBatch`). Since `protocol.CommitBatch` already serializes the way the JSON wire does, `traceToJson` = `decodeCommitBatch` then `std.json.Stringify.valueAlloc(gpa, decoded.batch, .{})`, dupe out, free the arena.

```zig
pub fn traceToJson(gpa: std.mem.Allocator, payload: []const u8) ![]u8 {
    var decoded = try decodeCommitBatch(gpa, payload);
    defer decoded.deinit();
    return std.json.Stringify.valueAlloc(gpa, decoded.batch, .{});
}
```

- [ ] **Step 5: Wire the test root in `build.zig`.** Immediately AFTER the `abi_tests` block (which ends with `test_step.dependOn(&b.addRunArtifact(abi_tests).step);` near line 189), add:

```zig
    // Binary NDP decoder tests (M10) — own root (Zig 0.16 doesn't collect
    // transitively). Reaches src/protocol.zig + src/generated/widget_types.zig
    // via same-directory / generated/ relative imports; no gobject.
    const binary_tests = b.addTest(.{
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/ndp_binary.zig"),
            .target = target,
            .optimize = optimize,
        }),
    });
    test_step.dependOn(&b.addRunArtifact(binary_tests).step);
```

- [ ] **Step 6: Run the wired tests.**

Run: `nix develop -c zig build test 2>&1 | tail -5`
Expected: PASS (all four `ndp_binary` tests run under `zig build test`; no other test regresses).

- [ ] **Step 7: Commit.**

```bash
git add src/ndp_binary.zig build.zig
git commit -m "feat(binary): zig CommitBatch decoder + tracer (spec §3-§9)"
```

---

## TASK 3 — TS binary `CommitBatch` encoder + outbox wiring

**Wave B (parallel; sole owner of `runtime/ndp.ts` + new `runtime/ndp-binary.ts`). Depends on: T5 (`packages/react/src/generated/widget-types.ts`).**

**Files:**
- Create: `runtime/ndp-binary.ts` (pure encoder)
- Create: `runtime/ndp-binary.test.ts` (golden-frame test, spec §8)
- Modify: `runtime/ndp.ts` (select encoding after handshake; encode `commitBatch` through the existing outbox)

**Interfaces:**
- Consumes: `WIDGET_TYPE` (T5), the `Op`/`CommitBatch` types in `runtime/ndp.ts`.
- Produces: `export function encodeCommitBatchBinary(batch: Omit<CommitBatch,"type">): Uint8Array` (the 28-byte header + op stream + string table `payload`, NOT frame-prefixed).
- `runtime/ndp.ts` gains a private `encoding: "json" | "binary"` set from `HelloAck.encodings` (pick first the runtime supports; binary preferred if offered).

**Critical fact:** `Bun.Socket.write()` does not buffer partial writes; `runtime/ndp.ts` already routes every frame through the drain-driven outbox (`send`→`outbox`→`pump`). Binary frames MUST go through `send()` too — do NOT add a second raw writer.

- [ ] **Step 1: Write the failing golden test.** Create `runtime/ndp-binary.test.ts` asserting the exact spec §8 bytes:

```typescript
import { test, expect } from "bun:test";
import { encodeCommitBatchBinary } from "./ndp-binary";

// ndp-binary spec §8 golden vector.
const batch = {
  commitId: 1,
  generation: 0,
  ops: [
    { op: "create", id: 1, widget: "Window", props: { title: "Hi" } },
    { op: "create", id: 2, widget: "Label", props: { text: "Clicks: 0" } },
    { op: "append", parent: 1, child: 2 },
    { op: "setText", id: 2, text: "Clicks: 1" },
  ],
} as const;

const expected = new Uint8Array([
  0x4e, 0x01, 0, 0, 0, 0, 0, 0,
  1, 0, 0, 0, 0, 0, 0, 0, // commitId=1
  0, 0, 0, 0, // generation
  4, 0, 0, 0, // opCount
  83, 0, 0, 0, // stringTableOffset (§8.4)
  0x01, 1, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0x04, 1, 0, 0, 0, // create Window
  0x01, 2, 0, 0, 0, 3, 0, 1, 0, 2, 0, 0, 0, 0x04, 3, 0, 0, 0, // create Label
  0x02, 1, 0, 0, 0, 2, 0, 0, 0, // append
  0x05, 2, 0, 0, 0, 4, 0, 0, 0, // setText
  5, 0, 0, 0,
  5, 0, 0, 0, 0x74, 0x69, 0x74, 0x6c, 0x65,
  2, 0, 0, 0, 0x48, 0x69,
  4, 0, 0, 0, 0x74, 0x65, 0x78, 0x74,
  9, 0, 0, 0, 0x43, 0x6c, 0x69, 0x63, 0x6b, 0x73, 0x3a, 0x20, 0x30,
  9, 0, 0, 0, 0x43, 0x6c, 0x69, 0x63, 0x6b, 0x73, 0x3a, 0x20, 0x31,
]);

test("binary encode matches spec §8 golden bytes", () => {
  const got = encodeCommitBatchBinary(batch);
  expect(Array.from(got)).toEqual(Array.from(expected));
});
```

- [ ] **Step 2: Run to verify it fails.**

Run: `nix develop -c bun test runtime/ndp-binary.test.ts`
Expected: FAIL — `Cannot find module './ndp-binary'`.

- [ ] **Step 3: Implement the encoder.** Create `runtime/ndp-binary.ts`. Intern strings first-seen in a `Map` (dedup mandatory, spec §6). Value tags per §5.3. Two-pass: build the op stream into a growable array, build the string table, then assemble header (with `stringTableOffset = 28 + opStream.length`).

```typescript
import { WIDGET_TYPE } from "../packages/react/src/generated/widget-types";

type Op =
  | { op: "create"; id: number; widget: string; props: Record<string, unknown> }
  | { op: "append"; parent: number; child: number }
  | { op: "insertBefore"; parent: number; child: number; before: number | null }
  | { op: "remove"; id: number }
  | { op: "setText"; id: number; text: string }
  | { op: "update"; id: number; props: Record<string, unknown> }
  | { op: "hide"; id: number }
  | { op: "unhide"; id: number };
type Batch = { commitId: number; generation: number; ops: Op[] };

class ByteWriter {
  private buf = new Uint8Array(256);
  len = 0;
  private ensure(n: number) {
    if (this.len + n <= this.buf.length) return;
    let cap = this.buf.length * 2;
    while (cap < this.len + n) cap *= 2;
    const next = new Uint8Array(cap);
    next.set(this.buf.subarray(0, this.len));
    this.buf = next;
  }
  u8(v: number) { this.ensure(1); this.buf[this.len++] = v & 0xff; }
  u16(v: number) { this.ensure(2); new DataView(this.buf.buffer).setUint16(this.len, v, true); this.len += 2; }
  u32(v: number) { this.ensure(4); new DataView(this.buf.buffer).setUint32(this.len, v >>> 0, true); this.len += 4; }
  i64(v: number) { this.ensure(8); new DataView(this.buf.buffer).setBigInt64(this.len, BigInt(v), true); this.len += 8; }
  f64(v: number) { this.ensure(8); new DataView(this.buf.buffer).setFloat64(this.len, v, true); this.len += 8; }
  bytes(b: Uint8Array) { this.ensure(b.length); this.buf.set(b, this.len); this.len += b.length; }
  slice(): Uint8Array { return this.buf.subarray(0, this.len); }
}

export function encodeCommitBatchBinary(batch: Batch): Uint8Array {
  const strings: string[] = [];
  const index = new Map<string, number>();
  const intern = (s: string): number => {
    const hit = index.get(s);
    if (hit !== undefined) return hit;
    const i = strings.length;
    strings.push(s);
    index.set(s, i);
    return i;
  };

  const ops = new ByteWriter();
  const writeProps = (props: Record<string, unknown>) => {
    const keys = Object.keys(props).filter((k) => k !== "testID"); // testID rides as meta, still a prop key on the wire? keep parity: encode all keys the JSON path sends.
    // Parity: the JSON path sends props verbatim (testID included). Encode ALL keys.
    const all = Object.keys(props);
    ops.u16(all.length);
    for (const k of all) {
      ops.u32(intern(k));
      const v = props[k];
      if (v === null || v === undefined) { ops.u8(0x00); }
      else if (typeof v === "boolean") { ops.u8(0x01); ops.u8(v ? 1 : 0); }
      else if (typeof v === "number") {
        if (Number.isInteger(v)) { ops.u8(0x02); ops.i64(v); }
        else { ops.u8(0x03); ops.f64(v); }
      } else if (typeof v === "string") { ops.u8(0x04); ops.u32(intern(v)); }
      else { throw new Error(`ndp-binary: unsupported prop value for "${k}" (${typeof v}) — spec §5.3 has no array/object tag`); }
    }
    void keys;
  };

  for (const op of batch.ops) {
    switch (op.op) {
      case "create": {
        ops.u8(0x01); ops.u32(op.id);
        const wt = WIDGET_TYPE[op.widget];
        if (wt === undefined) throw new Error(`ndp-binary: unknown widget "${op.widget}"`);
        ops.u16(wt); writeProps(op.props); break;
      }
      case "append": ops.u8(0x02); ops.u32(op.parent); ops.u32(op.child); break;
      case "insertBefore": ops.u8(0x03); ops.u32(op.parent); ops.u32(op.child); ops.u32(op.before ?? 0); break;
      case "remove": ops.u8(0x04); ops.u32(op.id); break;
      case "setText": ops.u8(0x05); ops.u32(op.id); ops.u32(intern(op.text)); break;
      case "update": ops.u8(0x06); ops.u32(op.id); writeProps(op.props); break;
      case "hide": ops.u8(0x07); ops.u32(op.id); break;
      case "unhide": ops.u8(0x08); ops.u32(op.id); break;
    }
  }

  const opBytes = ops.slice();
  const stringTableOffset = 28 + opBytes.length;

  const out = new ByteWriter();
  out.u8(0x4e); out.u8(0x01);
  for (let i = 0; i < 6; i++) out.u8(0);
  out.i64(batch.commitId); // commitId u64 — Number is safe for test-scale ids
  out.u32(batch.generation);
  out.u32(batch.ops.length);
  out.u32(stringTableOffset);
  out.bytes(opBytes);
  out.u32(strings.length);
  const enc = new TextEncoder();
  for (const s of strings) { const b = enc.encode(s); out.u32(b.length); out.bytes(b); }
  return out.slice().slice(); // copy out of the growable buffer
}
```

> **Documented deviation to verify against the Zig side:** the encoder writes `commitId` via `setBigInt64` (an `i64` view of the `u64` field). For all real commit ids this is identical bytes to a `u64` LE. Both sides read the same 8 LE bytes. No behavioral difference; noted because the field is `u64` in the spec but JS lacks a native u64.

- [ ] **Step 4: Run the golden test.**

Run: `nix develop -c bun test runtime/ndp-binary.test.ts`
Expected: PASS — bytes equal the spec §8 vector.

- [ ] **Step 5: Wire encoding selection into `runtime/ndp.ts`.** Add a private field and set it in `dispatch`'s helloAck arm; route `sendCommit` through binary when selected. Exact edits:

In the class field block (after `private outboxOffset = 0;`, line ~44):
```typescript
  // Negotiated CommitBatch encoding (ndp-binary spec §2): default "json";
  // "binary" only if the host advertises it in HelloAck.encodings AND this
  // runtime supports it. Fixed for the connection's lifetime.
  private encoding: "json" | "binary" = "json";
```

In `dispatch`, replace the helloAck arm (lines 111-113) with:
```typescript
    if (msg.type === "helloAck") {
      if (msg.ndpVersion !== NDP_VERSION) throw new Error(`ndp mismatch: host ${msg.ndpVersion}`);
      // Selection rule (spec §2): first host-advertised encoding this runtime
      // supports. This runtime supports "binary"; JSON is always the fallback.
      if (msg.encodings?.includes("binary")) this.encoding = "binary";
      this.helloAckResolve?.();
```

Replace `sendCommit` (line 142-144) with:
```typescript
  sendCommit(batch: Omit<CommitBatch, "type">): void {
    if (this.encoding === "binary") {
      this.sendBinaryFrame(encodeCommitBatchBinary(batch));
    } else {
      this.send({ type: "commitBatch", ...batch });
    }
  }

  /// Frames a raw binary payload (u32 LE length prefix + payload) and queues
  /// it through the SAME drain-driven outbox as JSON frames — Bun's socket
  /// does not buffer partial writes, so a large binary CommitBatch must never
  /// bypass the outbox (M5c fact).
  private sendBinaryFrame(payload: Uint8Array): void {
    const frame = new Uint8Array(4 + payload.length);
    new DataView(frame.buffer).setUint32(0, payload.length, true);
    frame.set(payload, 4);
    if (TRACE) console.error(`>> [binary commitBatch ${payload.length}B]`);
    const wasEmpty = this.outbox.length === 0;
    this.outbox.push(frame);
    if (wasEmpty) this.pump(this.socket);
  }
```

Add the import at the top of `runtime/ndp.ts`:
```typescript
import { encodeCommitBatchBinary } from "./ndp-binary";
```

- [ ] **Step 6: Verify the existing ndp.ts tests still pass (JSON default unchanged).**

Run: `nix develop -c bun test runtime/`
Expected: PASS — all of `runtime/ndp.test.ts` (JSON handshake, partial-frame, sendCommit) still green (they use a mock host advertising `["json"]`, so `encoding` stays `"json"`), plus the new binary golden test.

- [ ] **Step 7: Commit.**

```bash
git add runtime/ndp-binary.ts runtime/ndp-binary.test.ts runtime/ndp.ts
git commit -m "feat(binary): TS CommitBatch encoder + negotiated outbox wiring"
```

---

## TASK 6 — Capability ACL model (`src/acl.zig`)

**Wave B (parallel; disjoint file `src/acl.zig`; adds ONE build.zig test root — apply AFTER T2's build.zig edit). Depends on: nothing (pure model).**

**Files:**
- Create: `src/acl.zig`
- Modify: `build.zig` (add an `acl_tests` root after T2's `binary_tests` block)

**Interfaces:**
- Produces: `pub const Acl = struct { ... }` with `pub fn initDefault(gpa) Acl` (core UI ops granted, everything else denied), `pub fn parse(gpa, json: []const u8) !Acl` (grants manifest → Acl), `pub fn isAllowed(self, window_id: u32, permission: []const u8) bool`, `pub fn deinit(self)`.
- Consumed by: T7 (runtime dispatch) and T9 (plugin capability check).

**Design (architect-binding):** per-window grants, namespaced permissions. The eight `CommitBatch` op kinds map to core permissions: `create`→`core:window.create` for the `Window` widget kind, all other ops→`core:commit`. **Default policy grants `core:commit` and `core:window.create` for every window** (so existing demos never break); `plugin:*` and any explicitly-namespaced privileged permission default-deny. Grants manifest JSON shape:

```json
{ "grants": [ { "window": 0, "permissions": ["core:commit", "core:window.create", "plugin:hello.greet"] } ],
  "defaultWindow": ["core:commit", "core:window.create"] }
```

`window: 0` or `defaultWindow` = applies to all windows (the common case; the current single-window demos use one window). A permission is allowed if it appears in the window's explicit grant set OR the `defaultWindow` set.

- [ ] **Step 1: Write the failing test.** Create `src/acl.zig` with tests first:

```zig
const std = @import("std");

test "default policy grants core ops, denies plugin ops" {
    var acl = Acl.initDefault(std.testing.allocator);
    defer acl.deinit();
    try std.testing.expect(acl.isAllowed(0, "core:commit"));
    try std.testing.expect(acl.isAllowed(7, "core:window.create")); // any window
    try std.testing.expect(!acl.isAllowed(0, "plugin:hello.greet")); // default-deny
    try std.testing.expect(!acl.isAllowed(0, "core:fs.write")); // unknown privileged
}

test "parsed grants extend the default" {
    const json =
        \\{"defaultWindow":["core:commit","core:window.create"],
        \\ "grants":[{"window":0,"permissions":["plugin:hello.greet"]}]}
    ;
    var acl = try Acl.parse(std.testing.allocator, json);
    defer acl.deinit();
    try std.testing.expect(acl.isAllowed(0, "core:commit"));
    try std.testing.expect(acl.isAllowed(0, "plugin:hello.greet"));
    try std.testing.expect(!acl.isAllowed(0, "plugin:other.cmd"));
}

test "empty/malformed manifest falls back to safe default (core granted)" {
    var acl = try Acl.parse(std.testing.allocator, "not json");
    defer acl.deinit();
    try std.testing.expect(acl.isAllowed(0, "core:commit")); // never break demos
    try std.testing.expect(!acl.isAllowed(0, "plugin:x.y"));
}
```

- [ ] **Step 2: Run to verify it fails.**

Run: `nix develop -c zig test src/acl.zig`
Expected: FAIL — `Acl` undefined.

- [ ] **Step 3: Implement `Acl`.** Store an owned duplicate of every permission string (arena) so the manifest bytes need not outlive `parse`. A window-keyed hash map of permission sets, plus a default set.

```zig
const PermSet = std.StringHashMapUnmanaged(void);

pub const Acl = struct {
    arena: std.heap.ArenaAllocator,
    default_perms: PermSet = .{},
    per_window: std.AutoHashMapUnmanaged(u32, PermSet) = .{},

    pub fn initDefault(gpa: std.mem.Allocator) Acl {
        var self = Acl{ .arena = std.heap.ArenaAllocator.init(gpa) };
        const a = self.arena.allocator();
        // Core UI ops granted by default so every existing demo keeps working.
        self.default_perms.put(a, "core:commit", {}) catch {};
        self.default_perms.put(a, "core:window.create", {}) catch {};
        return self;
    }

    pub fn parse(gpa: std.mem.Allocator, json: []const u8) !Acl {
        var self = Acl.initDefault(gpa); // start from the safe default
        const a = self.arena.allocator();
        const parsed = std.json.parseFromSlice(std.json.Value, gpa, json, .{}) catch return self;
        defer parsed.deinit();
        const root = parsed.value;
        if (root != .object) return self;
        if (root.object.get("defaultWindow")) |dw| if (dw == .array) {
            for (dw.array.items) |item| if (item == .string) {
                self.default_perms.put(a, try a.dupe(u8, item.string), {}) catch {};
            };
        };
        if (root.object.get("grants")) |g| if (g == .array) {
            for (g.array.items) |grant| if (grant == .object) {
                const win: u32 = if (grant.object.get("window")) |w| (if (w == .integer) @intCast(w.integer) else 0) else 0;
                const perms = grant.object.get("permissions") orelse continue;
                if (perms != .array) continue;
                const gop = self.per_window.getOrPut(a, win) catch continue;
                if (!gop.found_existing) gop.value_ptr.* = .{};
                for (perms.array.items) |p| if (p == .string) {
                    gop.value_ptr.put(a, try a.dupe(u8, p.string), {}) catch {};
                };
            };
        };
        return self;
    }

    pub fn isAllowed(self: *Acl, window_id: u32, permission: []const u8) bool {
        if (self.default_perms.contains(permission)) return true;
        if (self.per_window.get(window_id)) |set| if (set.contains(permission)) return true;
        // window 0 grants apply to all windows (the single-window demo case).
        if (window_id != 0) if (self.per_window.get(0)) |set0| if (set0.contains(permission)) return true;
        return false;
    }

    pub fn deinit(self: *Acl) void { self.arena.deinit(); }
};
```

- [ ] **Step 4: Wire the test root in `build.zig`.** Immediately after T2's `binary_tests` block, add:

```zig
    const acl_tests = b.addTest(.{
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/acl.zig"),
            .target = target,
            .optimize = optimize,
        }),
    });
    test_step.dependOn(&b.addRunArtifact(acl_tests).step);
```

- [ ] **Step 5: Run.**

Run: `nix develop -c zig build test 2>&1 | tail -5`
Expected: PASS (three `acl` tests run; nothing else regresses).

- [ ] **Step 6: Commit.**

```bash
git add src/acl.zig build.zig
git commit -m "feat(acl): capability ACL model with default-safe policy"
```

---

## TASK 8 — `nd_plugin_v1` header + demo-plugin source skeleton

**Wave B (parallel; disjoint new files `include/nd_plugin.h` + `plugins/hello/`). Depends on: nothing. Note: `include/nd.h` is NOT touched here — T10 adds the `#include`.**

**Files:**
- Create: `include/nd_plugin.h`
- Create: `plugins/hello/plugin.zig` (the demo plugin's Zig source)

**Interfaces:**
- Produces (C): `nd_plugin_v1` struct (`abi_version`, `name`, capability declarations, `init`/`deinit`, command registration) + `nd_plugin_registry` (host callbacks a plugin uses to register commands). These are the frozen plugin ABI.
- Produces (Zig): `plugins/hello/plugin.zig` exporting `nd_plugin_entry` returning a `*const nd_plugin_v1`.

**Design (spec §9 + architect):** `nd_plugin_v1` is a semver'd struct: `abi_version` (must equal `ND_PLUGIN_ABI_VERSION = 1`), `name`, a NUL-terminated array of capability permission strings the plugin declares (checked against the ACL — a plugin command only runs if its declared permission is granted), and `init(registry)`/`deinit()`. `init` calls `registry->register_command(registry, "greet", handler)` to add an NDP-reachable command. **Minimum viable scope (architect):** the plugin registers a command reachable from JS and denied without its capability; full "manifest feeds codegen" is a documented follow-up (stated in T13's doc).

- [ ] **Step 1: Write `include/nd_plugin.h`.**

```c
/* include/nd_plugin.h — the nd_plugin_v1 native-plugin ABI (spec §9, D12).
   A native plugin is a C-ABI shared library exporting nd_plugin_entry(), which
   returns a pointer to a statically-lived nd_plugin_v1. Capability strings are
   checked against the NDP-dispatch ACL; a command runs only if its plugin's
   declared permission is granted for the window. Opt-in: nothing loads a
   plugin unless the embedder calls nd_load_plugin (include/nd.h). */
#ifndef ND_PLUGIN_H
#define ND_PLUGIN_H
#include <stdint.h>
#include <stddef.h>

#define ND_PLUGIN_ABI_VERSION 1

typedef struct nd_plugin_registry nd_plugin_registry;

/* A plugin command handler: receives the arg JSON (NUL-terminated) and writes
   a malloc'd result JSON to *result_out (freed by the core via nd_free).
   Returns 0 ok, negative JSON-RPC-style code on error. */
typedef int32_t (*nd_command_fn)(const char* arg_json, char** result_out);

/* Host callbacks a plugin uses from init(). register_command associates a
   command name (reachable over NDP as {"type":"pluginCommand","name",...})
   with a handler; the core namespaces it as plugin:<plugin-name>.<command>. */
struct nd_plugin_registry {
  void* host;   /* opaque core handle; pass back to the callbacks */
  void (*register_command)(nd_plugin_registry*, const char* command, nd_command_fn);
};

typedef struct nd_plugin_v1 {
  uint32_t abi_version;         /* must == ND_PLUGIN_ABI_VERSION */
  const char* name;             /* plugin identity, e.g. "hello" */
  const char* const* capabilities; /* NULL-terminated permission strings, e.g. {"plugin:hello.greet", NULL} */
  int32_t (*init)(nd_plugin_registry*);  /* register commands/widgets; 0 ok */
  void (*deinit)(void);
} nd_plugin_v1;

/* Every plugin shared library exports this symbol. */
typedef const nd_plugin_v1* (*nd_plugin_entry_fn)(void);

#endif /* ND_PLUGIN_H */
```

- [ ] **Step 2: Write `plugins/hello/plugin.zig`** — the first-party demo plugin. It declares `plugin:hello.greet`, registers a `greet` command that returns `{"greeting":"hello, <name>"}`. Mirrors the C struct with a Zig `extern struct` (layout must match `nd_plugin_v1`).

```zig
const std = @import("std");

const NdPluginRegistry = extern struct {
    host: ?*anyopaque,
    register_command: *const fn (*NdPluginRegistry, [*:0]const u8, NdCommandFn) callconv(.c) void,
};
const NdCommandFn = *const fn ([*:0]const u8, *?[*:0]u8) callconv(.c) i32;

const NdPluginV1 = extern struct {
    abi_version: u32,
    name: [*:0]const u8,
    capabilities: [*:null]const ?[*:0]const u8,
    init: *const fn (*NdPluginRegistry) callconv(.c) i32,
    deinit: *const fn () callconv(.c) void,
};

const caps = [_:null]?[*:0]const u8{"plugin:hello.greet"};

fn greet(arg_json: [*:0]const u8, result_out: *?[*:0]u8) callconv(.c) i32 {
    const arg = std.mem.span(arg_json);
    // Parse {"name":"..."} defensively; default to "world".
    var name: []const u8 = "world";
    const parsed = std.json.parseFromSlice(std.json.Value, std.heap.page_allocator, arg, .{}) catch null;
    if (parsed) |p| { defer p.deinit(); if (p.value == .object) if (p.value.object.get("name")) |n| if (n == .string) { name = n.string; }; }
    const out = std.fmt.allocPrintSentinel(std.heap.raw_c_allocator, "{{\"greeting\":\"hello, {s}\"}}", .{name}, 0) catch return -32603;
    result_out.* = out.ptr;
    return 0;
}

fn init(registry: *NdPluginRegistry) callconv(.c) i32 {
    registry.register_command(registry, "greet", &greet);
    return 0;
}
fn deinit() callconv(.c) void {}

const plugin = NdPluginV1{
    .abi_version = 1,
    .name = "hello",
    .capabilities = &caps,
    .init = &init,
    .deinit = &deinit,
};

pub export fn nd_plugin_entry() callconv(.c) *const NdPluginV1 {
    return &plugin;
}
```

> **Deviation note:** `greet` allocates its result with `std.heap.raw_c_allocator` (libc `malloc`) so the core's `nd_free` (`std.c.free`, `src/abi.zig:150`) releases it uniformly across languages — the same malloc/free convention `semantic_action` uses.

- [ ] **Step 3: Verify the header compiles as C and the plugin compiles as Zig.**

Run: `nix develop -c bash -c 'echo "#include \"include/nd_plugin.h\"" | cc -xc -fsyntax-only -Iinclude - && zig build-lib plugins/hello/plugin.zig -dynamic -femit-bin=/tmp/nd-hello-plugin-probe.so && echo PROBE_OK'`
Expected: `PROBE_OK` (header is valid C; plugin builds as a shared lib). The real build step is added in T9.

- [ ] **Step 4: Commit.**

```bash
git add include/nd_plugin.h plugins/hello/plugin.zig
git commit -m "feat(plugin): nd_plugin_v1 ABI header + hello demo plugin source"
```

### Interfaces (produced by this task)
- `include/nd_plugin.h` — the frozen `nd_plugin_v1` + `nd_plugin_registry` C ABI.
- `plugins/hello/plugin.zig` — a demo plugin declaring `plugin:hello.greet`, registering the `greet` command.

---

## TASK 7 — Wire binary routing + ACL enforcement into `runtime.zig`

**Wave C (sole owner of `src/runtime.zig`). Depends on: T2 (`src/ndp_binary.zig`), T6 (`src/acl.zig`).**

**Files:**
- Modify: `src/runtime.zig` (HelloAck encodings; frame-loop binary route; ACL check before apply)
- Modify: `src/abi.zig` (add `acl: ?*acl.Acl` field to `NdContext`, wired by T10's `nd_set_acl`; T7 only READS it — coordinate: T7 adds the field read via a getter that returns the default when unset, T10 adds the setter)

**Interfaces:**
- Consumes: `ndp_binary.isBinaryPayload`/`decodeCommitBatch`/`traceToJson` (T2), `acl.Acl.isAllowed` (T6).
- Produces: the host now advertises `encodings = &.{ "binary", "json" }`, decodes binary `commitBatch` frames, and denies ACL-violating commands with a structured `error` frame + `ND_ACL_DENY` marker.

**ACL mapping (architect-binding):** in the frame loop, a `commitBatch` requires `core:commit`; additionally, if any op is `create` with `widget == "Window"`, it requires `core:window.create`. A future `pluginCommand` frame (T9) requires `plugin:<name>.<command>`. Denials emit a structured NDP `error` frame and print `ND_ACL_DENY permission=<perm>` to stderr (testable headlessly).

- [ ] **Step 1: Write the failing test — ACL denies a window-create when not granted.** Add to `src/runtime.zig`'s test block (or a new `test` in the file — it has its own reachability via the `nd_hello_root` tests, but for a focused unit add it here and it runs under the exe tests root). Since `Runtime` needs a socket, test the **pure ACL-gate helper** instead of the full loop:

```zig
test "commitGate denies window.create without grant, allows core:commit" {
    var denied = acl.Acl.initDefault(std.testing.allocator); // grants both by default
    defer denied.deinit();
    // Build a minimal batch with a Window create.
    var ops = [_]protocol.Op{.{ .op = "create", .id = 1, .widget = "Window" }};
    const batch = protocol.CommitBatch{ .commitId = 0, .generation = 0, .ops = &ops };
    // Default policy grants core:window.create → allowed.
    try std.testing.expect(commitGate(&denied, batch) == null);

    // A restrictive ACL (no window.create) → denied with the permission name.
    var strict = acl.Acl{ .arena = std.heap.ArenaAllocator.init(std.testing.allocator) };
    defer strict.deinit();
    _ = strict.default_perms.put(strict.arena.allocator(), "core:commit", {}) catch {};
    try std.testing.expectEqualStrings("core:window.create", commitGate(&strict, batch).?);
}
```

- [ ] **Step 2: Run to verify it fails.**

Run: `nix develop -c zig build test 2>&1 | tail -5`
Expected: FAIL — `commitGate` undefined (and `acl`/`ndp_binary` imports missing).

- [ ] **Step 3: Add imports + `commitGate` to `src/runtime.zig`.** At the top with the other imports:

```zig
const acl = @import("acl.zig");
const ndp_binary = @import("ndp_binary.zig");
```

Add a free function (near `writeFrameOpts`):

```zig
/// Returns null if the batch is permitted; otherwise the first permission that
/// was denied (for the ND_ACL_DENY marker + error frame). Every commit needs
/// core:commit; a Window create additionally needs core:window.create.
fn commitGate(a: *acl.Acl, batch: protocol.CommitBatch) ?[]const u8 {
    if (!a.isAllowed(0, "core:commit")) return "core:commit";
    for (batch.ops) |op| {
        if (std.mem.eql(u8, op.op, "create")) {
            if (op.widget) |w| if (std.mem.eql(u8, w, "Window")) {
                if (!a.isAllowed(0, "core:window.create")) return "core:window.create";
            };
        }
    }
    return null;
}
```

- [ ] **Step 4: Advertise binary in the HelloAck.** Change line 179:

```zig
        self.writeFrame(protocol.HelloAck{ .ndpVersion = protocol.ndp_version, .encodings = &.{ "binary", "json" } });
```

- [ ] **Step 5: Route binary frames + enforce ACL in the frame loop.** Replace the `commitBatch` handling in `readerLoop` (lines 194-196). The reader can no longer rely solely on `peekType` (binary frames are not JSON) — sniff binary FIRST:

```zig
            if (ndp_binary.isBinaryPayload(bytes)) {
                if (trace) {
                    const j = ndp_binary.traceToJson(self.gpa, bytes) catch null;
                    if (j) |jj| { std.debug.print(">> {s}\n", .{jj}); self.gpa.free(jj); }
                }
                self.marshalBinaryCommit(bytes); // ownership transfers
                continue;
            }
            const kind = protocol.peekType(self.gpa, bytes) catch {
                self.gpa.free(bytes);
                continue;
            };
            defer self.gpa.free(kind);
            if (std.mem.eql(u8, kind, "commitBatch")) {
                self.marshalCommit(bytes); // ownership transfers (JSON path)
            } else if (std.mem.eql(u8, kind, "ping")) {
```

> Note: the existing `peekType` call at line 188 must be REMOVED from before the branch and folded into the JSON path above (the binary sniff happens on raw bytes, before any JSON parse). Keep the `ping`/`runtimeError`/else arms as-is.

- [ ] **Step 6: Add the binary marshal + ACL gate on both apply paths.** The ACL check runs on the UI thread inside apply (where the parsed batch exists), emitting the deny marker + error frame. Add `marshalBinaryCommit` and an ACL check inside both `applyOnUi` and a new `applyBinaryOnUi`. Factor the gate:

```zig
    /// True if allowed; on denial prints ND_ACL_DENY, sends a structured error
    /// frame, and returns false (the batch is dropped, not applied).
    fn checkAndApply(self: *Runtime, batch: protocol.CommitBatch) void {
        const the_acl = if (abi_backend.ctx.acl) |a| a else &default_acl;
        if (commitGate(the_acl, batch)) |denied| {
            std.debug.print("ND_ACL_DENY permission={s}\n", .{denied});
            self.writeFrame(.{ .type = "error", .message = "capability denied", .expected = @as(u32, 0), .got = @as(u32, 0) });
            return;
        }
        self.tree.apply(batch);
    }

    fn marshalBinaryCommit(self: *Runtime, bytes: []u8) void {
        const job = self.gpa.create(CommitJob) catch { self.gpa.free(bytes); return; };
        job.* = .{ .rt = self, .bytes = bytes };
        abi_backend.vtable.marshal_async(abi_backend.ctx, &applyBinaryOnUi, job);
    }

    fn applyBinaryOnUi(data: ?*anyopaque) callconv(.c) void {
        const job: *CommitJob = @ptrCast(@alignCast(data.?));
        const self = job.rt;
        var decoded = ndp_binary.decodeCommitBatch(self.gpa, job.bytes) catch {
            self.gpa.free(job.bytes); self.gpa.destroy(job); return;
        };
        defer decoded.deinit();
        self.checkAndApply(decoded.batch);
        self.gpa.free(job.bytes);
        self.gpa.destroy(job);
    }
```

Change `applyOnUi` (line 327) from `self.tree.apply(parsed.value);` to `self.checkAndApply(parsed.value);`.

Add a module-level default ACL (used when the embedder never called `nd_set_acl`):
```zig
var default_acl: acl.Acl = undefined;
var default_acl_ready: bool = false;
```
Initialize it in `Runtime.start` (after `self.dev = ...`):
```zig
        if (!default_acl_ready) { default_acl = acl.Acl.initDefault(gpa); default_acl_ready = true; }
```

- [ ] **Step 7: Add the `acl` field to `NdContext` in `src/abi.zig`.** (T10 owns the setter; T7 adds the field so `abi_backend.ctx.acl` compiles.) In `src/abi.zig`'s `NdContext` struct (lines 44-50), add:
```zig
    acl: ?*@import("acl.zig").Acl = null,
```

- [ ] **Step 8: Run the full gate — JSON path byte-identical, ACL test passes.**

Run:
```bash
nix develop -c bash -c 'zig build test 2>&1 | tail -5 && zig build && ./scripts/headless-m3.sh && ./scripts/headless-m5c.sh'
```
Expected: `commitGate` test PASSES; `headless-m3`/`headless-m5c` stay green (default ACL grants `core:commit`+`core:window.create`, so JSON demos are byte-identical — no `ND_ACL_DENY` appears).

- [ ] **Step 9: Commit.**

```bash
git add src/runtime.zig src/abi.zig
git commit -m "feat(acl,binary): NDP-dispatch ACL gate + binary frame routing"
```

---

## TASK 9 — Native-plugin loader + demo-plugin build

**Wave C (disjoint new file `src/plugin.zig` + demo build). Depends on: T8 (`include/nd_plugin.h`, `plugins/hello/plugin.zig`), T6 (`src/acl.zig`).**

**Files:**
- Create: `src/plugin.zig`
- Modify: `build.zig` (add a `nd-plugin-hello` shared-lib artifact + a `plugin_tests` root)
- Modify: `src/runtime.zig` (route a `pluginCommand` NDP frame — sole owner is T7, but this arm is small; **apply as a follow-on edit in T9 since T7 already committed** — coordinate: T9 adds ONLY the `pluginCommand` else-arm)

**Interfaces:**
- Produces: `pub const Registry = struct { commands: StringHashMap(...) }`; `pub fn load(gpa, path, acl_ptr) !*Loaded` (dlopen, resolve `nd_plugin_entry`, check `abi_version`, verify each declared capability is grantable, call `init(registry)`); `pub fn dispatch(name, arg_json) ?[]u8` (run a registered command's handler, malloc'd result).
- Consumed by: T7's `runtime.zig` `pluginCommand` arm and T10's `nd_load_plugin`.

**Design:** `dlopen` via `std.DynLib.open(path)`; `lib.lookup(nd_plugin_entry_fn, "nd_plugin_entry")`. Verify `abi_version == 1` (fail loud otherwise — `ND_PLUGIN_ABI_MISMATCH`). For each capability string, the plugin command is only dispatchable if the ACL grants it (checked at dispatch time against the current window). The registry maps `command` → handler; a `pluginCommand` NDP frame `{ "type":"pluginCommand","plugin":"hello","command":"greet","arg":{...} }` routes here.

- [ ] **Step 1: Write the failing test — load the demo plugin, dispatch greet.** The test needs the built `.so`; guard it to run only when the path env is set (CI passes it), else skip. Add to `src/plugin.zig`:

```zig
const std = @import("std");
const acl = @import("acl.zig");

test "load hello plugin and dispatch greet" {
    const path = std.posix.getenv("ND_TEST_PLUGIN_SO") orelse return error.SkipZigTest;
    const gpa = std.testing.allocator;
    var acl_grant = try acl.Acl.parse(gpa,
        "{\"grants\":[{\"window\":0,\"permissions\":[\"plugin:hello.greet\"]}]}");
    defer acl_grant.deinit();
    var loaded = try load(gpa, path, &acl_grant);
    defer loaded.deinit();
    const result = loaded.dispatch("greet", "{\"name\":\"m10\"}") orelse return error.NoResult;
    defer std.c.free(result.ptr);
    try std.testing.expect(std.mem.indexOf(u8, result, "hello, m10") != null);
}
```

> `std.posix.getenv` is gone in 0.16 for the general case, but a test binary reading its own env is fine via `std.process.getEnvVarOwned` — **grep the installed std first**; if `std.posix.getenv` is unavailable, use `std.process.getEnvVarOwned(gpa, "ND_TEST_PLUGIN_SO")` and free it. Adjust the test accordingly before running.

- [ ] **Step 2: Run to verify it fails.**

Run: `nix develop -c zig build test 2>&1 | tail -5`
Expected: FAIL — `load` undefined (test skips only once compilation succeeds; it won't compile yet).

- [ ] **Step 3: Implement `src/plugin.zig`.** Mirror the `nd_plugin_v1`/`nd_plugin_registry` layout from `include/nd_plugin.h` as Zig `extern struct`s (same fields, same order). Store the loaded lib + registry so handlers stay resident.

```zig
const NdCommandFn = *const fn ([*:0]const u8, *?[*:0]u8) callconv(.c) i32;

const NdPluginRegistry = extern struct {
    host: ?*anyopaque,
    register_command: *const fn (*NdPluginRegistry, [*:0]const u8, NdCommandFn) callconv(.c) void,
};
const NdPluginV1 = extern struct {
    abi_version: u32,
    name: [*:0]const u8,
    capabilities: [*:null]const ?[*:0]const u8,
    init: *const fn (*NdPluginRegistry) callconv(.c) i32,
    deinit: *const fn () callconv(.c) void,
};
const EntryFn = *const fn () callconv(.c) *const NdPluginV1;

// Single-plugin registry (v1). A real multi-plugin host keys by plugin name;
// v1 loads one demo plugin, so a flat command map suffices.
var g_commands: std.StringHashMapUnmanaged(NdCommandFn) = .{};
var g_gpa: std.mem.Allocator = undefined;
var g_registry: NdPluginRegistry = undefined;

fn registerCommandC(_: *NdPluginRegistry, command: [*:0]const u8, fn_ptr: NdCommandFn) callconv(.c) void {
    const name = g_gpa.dupe(u8, std.mem.span(command)) catch return;
    g_commands.put(g_gpa, name, fn_ptr) catch {};
}

pub const Loaded = struct {
    lib: std.DynLib,
    plugin: *const NdPluginV1,
    pub fn deinit(self: *Loaded) void {
        self.plugin.deinit();
        self.lib.close();
    }
    /// Runs a registered command; returns a malloc'd JSON result the CALLER
    /// frees with std.c.free (the plugin allocated it with libc malloc).
    pub fn dispatch(self: *Loaded, command: []const u8, arg_json: []const u8) ?[]u8 {
        _ = self;
        const handler = g_commands.get(command) orelse return null;
        const argz = g_gpa.dupeZ(u8, arg_json) catch return null;
        defer g_gpa.free(argz);
        var out: ?[*:0]u8 = null;
        if (handler(argz, &out) != 0) return null;
        const o = out orelse return null;
        return std.mem.span(o);
    }
};

pub fn load(gpa: std.mem.Allocator, path: []const u8, acl_ptr: *acl.Acl) !*Loaded {
    g_gpa = gpa;
    var lib = try std.DynLib.open(path);
    errdefer lib.close();
    const entry = lib.lookup(EntryFn, "nd_plugin_entry") orelse return error.NoPluginEntry;
    const plugin = entry();
    if (plugin.abi_version != 1) {
        std.debug.print("ND_PLUGIN_ABI_MISMATCH got={d} want=1\n", .{plugin.abi_version});
        return error.AbiMismatch;
    }
    // Verify every declared capability is grantable (else the plugin's
    // commands could never run — fail loud at load, spec §9).
    var i: usize = 0;
    while (plugin.capabilities[i]) |cap| : (i += 1) {
        const cap_s = std.mem.span(cap);
        if (!acl_ptr.isAllowed(0, cap_s)) {
            std.debug.print("ND_PLUGIN_CAP_DENIED name={s} cap={s}\n", .{ std.mem.span(plugin.name), cap_s });
            return error.CapabilityDenied;
        }
    }
    g_registry = .{ .host = null, .register_command = &registerCommandC };
    if (plugin.init(&g_registry) != 0) return error.PluginInitFailed;
    const loaded = try gpa.create(Loaded);
    loaded.* = .{ .lib = lib, .plugin = plugin };
    std.debug.print("ND_PLUGIN_LOADED name={s}\n", .{std.mem.span(plugin.name)});
    return loaded;
}
```

- [ ] **Step 4: Add the demo-plugin build artifact + test root to `build.zig`.** After the `libnd` step (near line 207), add the shared lib:

```zig
    // First-party demo plugin (M10): a C-ABI shared lib exporting
    // nd_plugin_entry. Built as its own artifact; headless-m10.sh dlopens it.
    const plugin_hello = b.addLibrary(.{
        .name = "nd_plugin_hello",
        .linkage = .dynamic,
        .root_module = b.createModule(.{
            .root_source_file = b.path("plugins/hello/plugin.zig"),
            .target = target,
            .optimize = optimize,
            .link_libc = true,
        }),
    });
    const plugin_step = b.step("plugin-hello", "Build the hello demo plugin (.so)");
    plugin_step.dependOn(&b.addInstallArtifact(plugin_hello, .{}).step);
```

And the test root (after T6's `acl_tests` block):

```zig
    const plugin_tests = b.addTest(.{
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/plugin.zig"),
            .target = target,
            .optimize = optimize,
            .link_libc = true,
        }),
    });
    test_step.dependOn(&b.addRunArtifact(plugin_tests).step);
```

- [ ] **Step 5: Add the `pluginCommand` NDP arm to `src/runtime.zig`.** In the frame loop, after the `runtimeError` arm and before the `else`, add (this is T9's only edit to runtime.zig):

```zig
            } else if (std.mem.eql(u8, kind, "pluginCommand")) {
                self.handlePluginCommand(bytes);
```

And the handler method (mirror `marshalCommit`'s ownership, but plugin dispatch is cheap and safe off the UI thread — it touches only the registry; run it inline). It gates on `plugin:<plugin>.<command>` via the ACL and replies with a `pluginResult` frame:

```zig
    fn handlePluginCommand(self: *Runtime, bytes: []u8) void {
        defer self.gpa.free(bytes);
        const PC = struct { plugin: []const u8 = "", command: []const u8 = "", arg: std.json.Value = .null };
        const parsed = std.json.parseFromSlice(PC, self.gpa, bytes, .{ .ignore_unknown_fields = true }) catch return;
        defer parsed.deinit();
        const perm = std.fmt.allocPrint(self.gpa, "plugin:{s}.{s}", .{ parsed.value.plugin, parsed.value.command }) catch return;
        defer self.gpa.free(perm);
        const the_acl = if (abi_backend.ctx.acl) |a| a else &default_acl;
        if (!the_acl.isAllowed(0, perm)) {
            std.debug.print("ND_ACL_DENY permission={s}\n", .{perm});
            self.writeFrame(.{ .type = "error", .message = "capability denied", .expected = @as(u32, 0), .got = @as(u32, 0) });
            return;
        }
        const loaded = abi_backend.ctx.plugin orelse { self.writeFrame(.{ .type = "error", .message = "no plugin loaded", .expected = @as(u32, 0), .got = @as(u32, 0) }); return; };
        const arg_json = std.json.Stringify.valueAlloc(self.gpa, parsed.value.arg, .{}) catch return;
        defer self.gpa.free(arg_json);
        if (loaded.dispatch(parsed.value.command, arg_json)) |result| {
            defer std.c.free(result.ptr);
            std.debug.print("ND_PLUGIN_COMMAND_OK plugin={s} command={s}\n", .{ parsed.value.plugin, parsed.value.command });
            // Forward the plugin's JSON result as a pluginResult frame (result is a JSON string).
            var buf: [256]u8 = undefined;
            const framed = std.fmt.bufPrint(&buf, "{{\"type\":\"pluginResult\",\"result\":{s}}}", .{result}) catch return;
            self.writeRawJson(framed);
        }
    }
```

> `abi_backend.ctx.plugin` is the `?*plugin.Loaded` field T10 adds to `NdContext`. `writeRawJson` is a small helper: frame a raw JSON string through the writer mutex (add it next to `writeFrameOpts`, mirroring its lock/flush but taking a pre-serialized `[]const u8` payload). If a `writeRawJson` already feels redundant, serialize a typed struct instead — but the plugin result is arbitrary JSON, so the raw path is simplest.

- [ ] **Step 6: Build the plugin and run the plugin test with the .so path.**

Run:
```bash
nix develop -c bash -c 'zig build plugin-hello && ND_TEST_PLUGIN_SO=$(pwd)/zig-out/lib/libnd_plugin_hello.so zig build test 2>&1 | tail -8'
```
Expected: the demo `.so` builds; the `load hello plugin and dispatch greet` test PASSES (finds "hello, m10").

- [ ] **Step 7: Commit.**

```bash
git add src/plugin.zig build.zig src/runtime.zig
git commit -m "feat(plugin): dlopen loader + pluginCommand dispatch (ACL-gated)"
```

---

## TASK 10 — `include/nd.h` additions + `nd_set_acl`/`nd_load_plugin` exports

**Wave C (sole owner of `include/nd.h` + the export additions to `src/abi.zig`). Depends on: T6 (acl), T9 (plugin), T7 (NdContext.acl field already added).**

**Files:**
- Modify: `include/nd.h` (add `#include "nd_plugin.h"`; add `nd_set_acl`/`nd_load_plugin` prototypes)
- Modify: `src/abi.zig` (add `plugin: ?*plugin.Loaded` field to `NdContext`; export `nd_set_acl`/`nd_load_plugin`)

**Interfaces:**
- Produces (C): `void nd_set_acl(nd_context*, const char* grants_json);` and `int32_t nd_load_plugin(nd_context*, const char* path);`.
- **No `nd_backend` vtable change** — the 18-field vtable + its `@sizeOf` assert are untouched. These are lifecycle functions, exactly like `nd_start_runtime`.

- [ ] **Step 1: Write the failing test — the ABI layout asserts + new exports compile.** Add to `src/abi.zig`'s existing test surface (it already has comptime asserts). Add:

```zig
test "nd_set_acl parses grants into the context" {
    const self = nd_init().?;
    defer std.heap.page_allocator.destroy(self);
    nd_set_acl(self, "{\"grants\":[{\"window\":0,\"permissions\":[\"plugin:hello.greet\"]}]}");
    try std.testing.expect(self.acl != null);
    try std.testing.expect(self.acl.?.isAllowed(0, "plugin:hello.greet"));
}
```

- [ ] **Step 2: Run to verify it fails.**

Run: `nix develop -c zig build test 2>&1 | tail -5`
Expected: FAIL — `nd_set_acl` undefined.

- [ ] **Step 3: Add the header prototypes.** In `include/nd.h`, after `#include <stdbool.h>` add:
```c
#include "nd_plugin.h"
```
And in the lifecycle section (after `nd_start_automation`, line 61):
```c
/* Capability ACL (D12): install a per-window grants manifest (NUL-terminated
   JSON; see docs). Absent = safe default (core UI ops granted, plugin ops
   denied). Call before nd_start_runtime. */
void nd_set_acl(nd_context*, const char* grants_json);
/* Load a native nd_plugin_v1 shared library (opt-in). Returns 0 ok, negative
   on ABI mismatch / missing entry / capability-denied. */
int32_t nd_load_plugin(nd_context*, const char* path);
```

- [ ] **Step 4: Add the field + exports to `src/abi.zig`.** In `NdContext` (T7 already added `acl`), add:
```zig
    plugin: ?*@import("plugin.zig").Loaded = null,
```
Add the imports at the top: `const acl = @import("acl.zig");` and `const plugin = @import("plugin.zig");`. Then the exports (after `nd_start_automation`):

```zig
pub export fn nd_set_acl(self: *NdContext, grants_json: [*:0]const u8) callconv(.c) void {
    const json = std.mem.span(grants_json);
    const a = self.gpa.create(acl.Acl) catch return;
    a.* = acl.Acl.parse(self.gpa, json) catch acl.Acl.initDefault(self.gpa);
    self.acl = a;
}

pub export fn nd_load_plugin(self: *NdContext, path: [*:0]const u8) callconv(.c) i32 {
    const p = std.mem.span(path);
    const acl_ptr = self.acl orelse blk: {
        const a = self.gpa.create(acl.Acl) catch return -1;
        a.* = acl.Acl.initDefault(self.gpa);
        self.acl = a;
        break :blk a;
    };
    const loaded = plugin.load(self.gpa, p, acl_ptr) catch return -1;
    self.plugin = loaded;
    return 0;
}
```

- [ ] **Step 5: Force-retain the new exports in `src/core/root.zig`.** Add to the `comptime` block (lines 26-33):
```zig
    _ = &abi.nd_set_acl;
    _ = &abi.nd_load_plugin;
```

- [ ] **Step 6: Run.**

Run: `nix develop -c bash -c 'zig build test 2>&1 | tail -5 && zig build libnd 2>&1 | tail -3'`
Expected: PASS (the `nd_set_acl` test passes; the `@sizeOf(NdBackend) == 18 * @sizeOf(usize)` assert still holds — vtable unchanged); `libnd` builds with the two new symbols present.

- [ ] **Step 7: Verify the header is still valid C.**

Run: `nix develop -c bash -c 'echo "#include \"include/nd.h\"" | cc -xc -fsyntax-only -Iinclude - && echo NDH_OK'`
Expected: `NDH_OK`.

- [ ] **Step 8: Commit.**

```bash
git add include/nd.h src/abi.zig src/core/root.zig
git commit -m "feat(plugin,acl): nd_set_acl + nd_load_plugin lifecycle exports"
```

---

## TASK 4 — 10k-node benchmark driver (JSON + binary legs)

**Wave C (disjoint new file `scripts/bench-10k.ts`). Depends on: T3 (binary encoder wired into `runtime/ndp.ts`).**

**Files:**
- Create: `scripts/bench-10k.ts`

**Interfaces:**
- Consumes: `runtime/ndp.ts` `Ndp` (connect/handshake/sendCommit). It is a plain NDP child (like `runtime/m2-demo.ts`) that builds a 10k-node tree and prints timing markers.
- Produces: markers `ND_BENCH_MOUNT encoding=<json|binary> nodes=10000 ms=<n>` and `ND_BENCH_DONE`.

**Design (architect-binding):** the "gate" = binary must not be slower than JSON; timings are informational; pass/fail is completion within a generous bound. The harness runs headless (weston) via `scripts/headless-m10.sh` (T14). This task delivers the driver; T14 wires the two legs (one host run negotiating json, one negotiating binary) and the comparison.

**Encoding selection:** the driver cannot choose the encoding itself — the HOST advertises it. So the two legs differ by which host build/flag is active: the host always advertises `["binary","json"]` (T7), so the runtime picks binary. To force the JSON leg, set an env `ND_FORCE_JSON=1` the runtime reads to ignore `"binary"` in `HelloAck.encodings`. **Add that env check to `runtime/ndp.ts`'s helloAck arm** (T3 owns ndp.ts, but this is a 1-line addition — coordinate: if T3 is already committed, add it here as a small ndp.ts edit within this task's commit):

```typescript
      if (msg.encodings?.includes("binary") && process.env.ND_FORCE_JSON !== "1") this.encoding = "binary";
```

- [ ] **Step 1: Write `scripts/bench-10k.ts`.** Build 10k nodes: one Window, one Box, then 9998 Labels appended to the Box, in a single CommitBatch. Time the encode+send, and rely on the host's `ND_COMMIT_APPLIED` (printed by `tree.apply`) to bound completion. Print the driver-side encode/send time as the informational marker.

```typescript
// 10k-node mount benchmark driver (M10 gate). Builds a 10k-node tree in ONE
// CommitBatch and reports encode+send time. The host prints ND_COMMIT_APPLIED
// once tree.apply finishes; headless-m10.sh (the harness) bounds total wall
// time and compares the json vs binary legs by poll-count/marker, not by
// flaky wall-clock. Encoding is chosen by the host handshake (ND_FORCE_JSON=1
// forces the json leg).
import { Ndp } from "../runtime/ndp";

const N = 10000;
const encoding = process.env.ND_FORCE_JSON === "1" ? "json" : "binary";

const ndp = await Ndp.connect();
await ndp.handshake({ name: "bun", version: Bun.version });

const ops: any[] = [];
ops.push({ op: "create", id: 1, widget: "Window", props: { title: "bench" } });
ops.push({ op: "create", id: 2, widget: "Box", props: { orientation: "vertical", spacing: 0 } });
ops.push({ op: "append", parent: 1, child: 2 });
for (let i = 3; i <= N; i++) {
  ops.push({ op: "create", id: i, widget: "Label", props: { text: `row ${i}` } });
  ops.push({ op: "append", parent: 2, child: i });
}

const t0 = performance.now();
ndp.sendCommit({ commitId: 0, generation: 0, ops });
const ms = (performance.now() - t0).toFixed(1);
console.error(`ND_BENCH_MOUNT encoding=${encoding} nodes=${N} ms=${ms}`);

// Keep the process alive briefly so the host applies the commit before exit.
await Bun.sleep(2000);
console.error("ND_BENCH_DONE");
process.exit(0);
```

- [ ] **Step 2: Smoke-run the driver against a headless host (JSON leg).**

Run:
```bash
nix develop -c bash -c '
  export XDG_RUNTIME_DIR="$(mktemp -d)"; export WAYLAND_DISPLAY=nd-bench-probe; export GSK_RENDERER=cairo GDK_BACKEND=wayland
  weston --backend=headless --socket="$WAYLAND_DISPLAY" --idle-time=0 & WP=$!
  for _ in $(seq 1 50); do [ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ] && break; sleep 0.1; done
  LOG=$(mktemp); ND_FORCE_JSON=1 ND_SCRIPT=scripts/bench-10k.ts ./zig-out/bin/nd-hello >"$LOG" 2>&1 &
  HP=$!; sleep 4; grep -E "ND_BENCH_MOUNT|ND_COMMIT_APPLIED" "$LOG" | tail -3; kill $HP $WP 2>/dev/null'
```
Expected: `ND_BENCH_MOUNT encoding=json nodes=10000 ms=<n>` and `ND_COMMIT_APPLIED commitId=0` both appear.

- [ ] **Step 3: Smoke-run the binary leg** (drop `ND_FORCE_JSON`): same command without `ND_FORCE_JSON=1`.
Expected: `ND_BENCH_MOUNT encoding=binary ...` + `ND_COMMIT_APPLIED`. (This is the first end-to-end proof the binary decoder handles a 10k-node tree.)

- [ ] **Step 4: Add the `ND_FORCE_JSON` line to `runtime/ndp.ts`** (if not already present from T3's edit) and re-run `bun test runtime/` to confirm no regression.

- [ ] **Step 5: Commit.**

```bash
git add scripts/bench-10k.ts runtime/ndp.ts
git commit -m "feat(bench): 10k-node mount driver (json+binary legs)"
```

---

## TASK 11 — Swift shell wiring for the new C ABI (Mac stays green)

**Wave C (sole owner of `swift/Sources/NDShell/main.swift` for this milestone). Depends on: T10 (`include/nd.h` new prototypes).**

**Files:**
- Modify: `swift/Sources/NDShell/main.swift`

**Interfaces:**
- Consumes: the new `nd_set_acl`/`nd_load_plugin` C functions (via `CNd`), and `include/nd_plugin.h` (now included from `nd.h`, so `CNd` re-exports it).
- Produces: the Mac shell optionally calls `nd_set_acl` / `nd_load_plugin` when env is set; the default (env unset) is byte-identical to today's behavior.

**Why:** `include/nd.h` changed shape (new include + two functions). The `CNd` module map re-exports everything from `nd.h`, so `nd_plugin.h`'s types now appear in Swift too. The shell must still compile and its default run must be unchanged. This is the required Swift-side wiring + `ssh macbook` verification leg (Global Constraints).

- [ ] **Step 1: Add opt-in ACL + plugin calls to `main.swift`.** After `gVTable = buildVTable()`/`nd_register_backend(ctx, &gVTable)` and BEFORE `nd_start_runtime(ctx)`:

```swift
// M10: opt-in capability ACL + native plugin. Absent env = safe default
// (core UI ops granted), byte-identical to pre-M10 behavior.
if let grants = ProcessInfo.processInfo.environment["ND_ACL_GRANTS"] {
    grants.withCString { nd_set_acl(ctx, $0) }
}
if ProcessInfo.processInfo.environment["ND_PLUGINS"] == "1",
   let pluginPath = ProcessInfo.processInfo.environment["ND_PLUGIN_PATH"] {
    let rc = pluginPath.withCString { nd_load_plugin(ctx, $0) }
    if rc != 0 { FileHandle.standardError.write("ND_PLUGIN_LOAD_FAILED rc=\(rc)\n".data(using: .utf8)!) }
}
```

- [ ] **Step 2: Verify the Swift shell still builds on the Mac + the counter run is unchanged (over ssh).**

Run (mirrors `scripts/mac/mac-m6.sh`'s heredoc + repack recipe):
```bash
./scripts/mac/mac-sync.sh
ssh macbook 'bash -euo pipefail -s' <<'REMOTE'
export PATH="/etc/profiles/per-user/kyandesutter/bin:$PATH"
cd ~/nd
zig build libnd -Dbackend=abi >/dev/null 2>&1
workdir="$(mktemp -d)"; ( cd "$workdir" && ar x ~/nd/zig-out/lib/libnd.a && chmod 644 *.o && libtool -static -o ~/nd/zig-out/lib/libnd.a *.o ); rm -rf "$workdir"
( cd swift && swift build -c release 2>&1 | tail -5 )
LOG=/tmp/nd-m10-swift.log; rm -f "$LOG"
ND_SCRIPT=examples/counter/main.tsx NATIVE_AUTOMATION=1 timeout 8s swift/.build/release/NDShell >"$LOG" 2>&1 || true
grep -q ND_COMMIT_APPLIED "$LOG" && echo "M10_SWIFT_OK" || { echo "FAIL: swift shell"; cat "$LOG"; exit 1; }
REMOTE
```
Expected: `swift build` succeeds (new `nd_set_acl`/`nd_load_plugin` symbols resolve from `libnd.a`); `M10_SWIFT_OK` (default run unchanged — no ACL env set, so binary/ACL/plugins are inert on the Mac path).

- [ ] **Step 3: Commit.**

```bash
git add swift/Sources/NDShell/main.swift
git commit -m "feat(mac): wire nd_set_acl + nd_load_plugin (opt-in, default inert)"
```

---

## TASK 12 — Bun lifecycle-script blocking (D12 hardening extra)

**Wave D. Depends on: nothing.**

**Files:**
- Modify: `template/package.json` (add `trustedDependencies` if the file exists and it's trivial)

**Interfaces:** none (config hardening only).

**Design (architect):** the CLI surface is minimal (template + scripts). D12 wants Bun's lifecycle-script blocking preserved. Bun blocks postinstall scripts by default unless a dependency is listed in `trustedDependencies`. Scope: assert the template does not opt-in untrusted lifecycle scripts, and add an explicit empty `trustedDependencies: []` as documentation-of-intent if the file exists; else defer with a note.

- [ ] **Step 1: Check the template exists and inspect it.**

Run: `test -f template/package.json && cat template/package.json || echo "NO_TEMPLATE_PKG"`

- [ ] **Step 2a (if `template/package.json` exists):** Add `"trustedDependencies": []` (explicit empty allowlist = "no dependency may run lifecycle scripts", Bun's safe default made explicit). Insert it as a top-level key.

- [ ] **Step 2b (if it does NOT exist):** Skip the edit. Record in the T14 activeContext update: "D12 Bun lifecycle-script blocking: template has no package.json / non-trivial — deferred; Bun blocks postinstall by default, no regression."

- [ ] **Step 3: Verify (if edited) the JSON is valid.**

Run: `test -f template/package.json && nix develop -c bun -e 'JSON.parse(require("fs").readFileSync("template/package.json","utf8")); console.log("PKG_OK")' || echo "SKIPPED"`
Expected: `PKG_OK` or `SKIPPED`.

- [ ] **Step 4: Commit (only if edited).**

```bash
git add template/package.json
git commit -m "chore(cli): explicit empty trustedDependencies (D12 lifecycle-script blocking)"
```

---

## TASK 13 — WASM plugin-tier deferral design doc

**Wave D. Depends on: T8 (references the `nd_plugin_v1` surface).**

**Files:**
- Create: `docs/superpowers/specs/2026-07-11-wasm-plugin-tier.md`

**Interfaces:** none (documentation).

**Design (architect-binding, owner-visible descope):** wasmtime/Extism is NOT integrated this milestone. Document the deferral rationale and how the WASM tier slots behind the same `nd_plugin_v1` surface as a second loader.

- [ ] **Step 1: Write the doc.** Cover: (1) status = deferred, owner-visible descope; (2) rationale — wasmtime is a heavy Rust/C dependency not present in the nix devshell, and the 2026 wasmtime CVEs (spec §9) show the host-function surface is the real sandbox boundary, so it deserves its own milestone; (3) the architecture that makes it a drop-in: the WASM tier is a **second loader behind `nd_plugin_v1`** — instead of `dlopen`+`nd_plugin_entry`, a WASM loader instantiates an Extism plugin and adapts its exported functions to the same `nd_plugin_registry.register_command` surface, with the SAME ACL capability check (a WASM plugin declares capabilities identically and is denied identically); (4) the minimal host-function surface principle (only `register_command`, no ambient filesystem/network); (5) the "manifest feeds codegen" follow-up (spec §9: a plugin metadata manifest feeds `tools/codegen.ts` so TS types for plugin commands appear automatically — not built in M10; the demo plugin's command is reachable via the untyped `pluginCommand` NDP frame today).

- [ ] **Step 2: Verify the doc references the real surface.**

Run: `rg -n 'nd_plugin_v1|register_command|pluginCommand|Extism|wasmtime' docs/superpowers/specs/2026-07-11-wasm-plugin-tier.md | head`
Expected: hits for each — the doc is grounded in the actual ABI, not hand-wavy.

- [ ] **Step 3: Commit.**

```bash
git add docs/superpowers/specs/2026-07-11-wasm-plugin-tier.md
git commit -m "docs(plugin): wasm tier deferral rationale + nd_plugin_v1 slot-in design"
```

---

## TASK 14 — Integration: `headless-m10.sh` + CI + activeContext

**Wave D (final). Depends on: T4, T7, T9, T10 (all landed).**

**Files:**
- Create: `scripts/headless-m10.sh`
- Modify: `.github/workflows/ci.yml` (add `bun test runtime/` + `headless-m10` steps)
- Modify: `CLAUDE-activeContext.md` (M10 done + hard-won facts)

**Interfaces:** none (harness + CI + docs).

`headless-m10.sh` has three legs, each a self-contained weston+host run with a UNIQUE `WAYLAND_DISPLAY` and marker-gated polling (model: `scripts/headless-m5c.sh`):
- **Leg 1 — benchmark:** run the JSON leg (`ND_FORCE_JSON=1`) and the binary leg of `scripts/bench-10k.ts`; assert both reach `ND_COMMIT_APPLIED` within the poll bound; parse both `ND_BENCH_MOUNT ... ms=` values; **gate = binary ms ≤ json ms × 2** (generous bound; record both numbers). The strict "not slower" is informational — a 2× ceiling avoids CI flakiness while proving binary is in the same class.
- **Leg 2 — ACL deny:** run the counter host with a restrictive ACL (`ND_ACL_GRANTS` withholding `core:window.create`); assert `ND_ACL_DENY permission=core:window.create` appears and NO `ND_COMMIT_APPLIED` (the window-creating batch is denied).
- **Leg 3 — plugin load:** build `plugin-hello`; run a host with `ND_PLUGINS=1 ND_PLUGIN_PATH=<abs .so>` + a grants manifest for `plugin:hello.greet`; a small child (`scripts/m10-plugin-drive.ts`, created here) sends a `pluginCommand` greet; assert `ND_PLUGIN_LOADED name=hello` + `ND_PLUGIN_COMMAND_OK plugin=hello command=greet`; then run it WITHOUT the grant and assert `ND_ACL_DENY permission=plugin:hello.greet`.

- [ ] **Step 1: Create `scripts/m10-plugin-drive.ts`** — a minimal NDP child that handshakes then sends one `pluginCommand`. It reuses `runtime/ndp.ts`'s socket but `pluginCommand` isn't a typed method, so send it raw via a tiny inline framing (or add a `sendRaw(obj)` public passthrough — simplest: add `ndp.sendPluginCommand(plugin, command, arg)` to `runtime/ndp.ts`). To avoid another `ndp.ts` edit this late, the driver opens the socket directly:

```typescript
// Minimal pluginCommand driver for headless-m10.sh leg 3.
const path = process.env.ND_SOCKET!;
const socket = await Bun.connect({ unix: path, socket: { data() {}, close() {} } });
function frame(obj: unknown): Uint8Array {
  const body = new TextEncoder().encode(JSON.stringify(obj));
  const f = new Uint8Array(4 + body.length);
  new DataView(f.buffer).setUint32(0, body.length, true); f.set(body, 4); return f;
}
socket.write(frame({ type: "hello", ndpVersion: 1, runtime: { name: "bun", version: Bun.version } }));
await Bun.sleep(300);
socket.write(frame({ type: "pluginCommand", plugin: "hello", command: "greet", arg: { name: "m10" } }));
await Bun.sleep(500);
console.error("M10_PLUGIN_DRIVE_DONE");
process.exit(0);
```

- [ ] **Step 2: Create `scripts/headless-m10.sh`.** Three legs, marker-gated, unique weston sockets. (Full script — model after `headless-m5c.sh`.)

```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

run_host() { # $1=display $2=extra-env-prefix $3=script $4=logfile
  export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
  export WAYLAND_DISPLAY="$1" GSK_RENDERER=cairo GDK_BACKEND=wayland
  weston --backend=headless --socket="$WAYLAND_DISPLAY" --idle-time=0 & echo $!
}

# ---- Leg 1: benchmark (json vs binary) ----
DISP=nd-m10-bench-json; export XDG_RUNTIME_DIR="$(mktemp -d)"; export WAYLAND_DISPLAY=$DISP GSK_RENDERER=cairo GDK_BACKEND=wayland
weston --backend=headless --socket="$DISP" --idle-time=0 & WP=$!
trap 'kill "$WP" ${HP:-} 2>/dev/null || true' EXIT
for _ in $(seq 1 50); do [ -S "$XDG_RUNTIME_DIR/$DISP" ] && break; sleep 0.1; done

JLOG=$(mktemp); ND_FORCE_JSON=1 ND_SCRIPT=scripts/bench-10k.ts ./zig-out/bin/nd-hello >"$JLOG" 2>&1 & HP=$!
for _ in $(seq 1 100); do grep -q ND_COMMIT_APPLIED "$JLOG" && break; sleep 0.1; done
grep -q ND_COMMIT_APPLIED "$JLOG" || { echo "FAIL: json bench no commit"; cat "$JLOG"; exit 1; }
kill "$HP" 2>/dev/null || true; wait "$HP" 2>/dev/null || true
JMS=$(grep -m1 ND_BENCH_MOUNT "$JLOG" | sed 's/.*ms=//')

BLOG=$(mktemp); ND_SCRIPT=scripts/bench-10k.ts ./zig-out/bin/nd-hello >"$BLOG" 2>&1 & HP=$!
for _ in $(seq 1 100); do grep -q ND_COMMIT_APPLIED "$BLOG" && break; sleep 0.1; done
grep -q ND_COMMIT_APPLIED "$BLOG" || { echo "FAIL: binary bench no commit"; cat "$BLOG"; exit 1; }
grep -q "ND_BENCH_MOUNT encoding=binary" "$BLOG" || { echo "FAIL: binary encoding not negotiated"; cat "$BLOG"; exit 1; }
kill "$HP" 2>/dev/null || true; wait "$HP" 2>/dev/null || true
BMS=$(grep -m1 ND_BENCH_MOUNT "$BLOG" | sed 's/.*ms=//')
echo "M10_BENCH json_ms=$JMS binary_ms=$BMS"
# Gate: binary must be in the same class (<= 2x json). Informational + bounded.
awk -v b="$BMS" -v j="$JMS" 'BEGIN{ if (b > j*2 + 5) { print "FAIL: binary slower than 2x json"; exit 1 } }'

# ---- Leg 2: ACL deny ----
DISP=nd-m10-acl; export WAYLAND_DISPLAY=$DISP
weston --backend=headless --socket="$DISP" --idle-time=0 & WP2=$!
for _ in $(seq 1 50); do [ -S "$XDG_RUNTIME_DIR/$DISP" ] && break; sleep 0.1; done
ALOG=$(mktemp)
ND_ACL_GRANTS='{"defaultWindow":["core:commit"]}' ND_SCRIPT=examples/counter/main.tsx timeout 8s ./zig-out/bin/nd-hello >"$ALOG" 2>&1 || true
grep -q "ND_ACL_DENY permission=core:window.create" "$ALOG" || { echo "FAIL: no ACL deny"; cat "$ALOG"; exit 1; }
echo "M10_ACL_DENY_OK"
kill "$WP2" 2>/dev/null || true

# ---- Leg 3: plugin load + gated dispatch ----
DISP=nd-m10-plugin; export WAYLAND_DISPLAY=$DISP
weston --backend=headless --socket="$DISP" --idle-time=0 & WP3=$!
for _ in $(seq 1 50); do [ -S "$XDG_RUNTIME_DIR/$DISP" ] && break; sleep 0.1; done
SO="$(pwd)/zig-out/lib/libnd_plugin_hello.so"
[ -f "$SO" ] || { echo "FAIL: build zig build plugin-hello first"; exit 1; }
PLOG=$(mktemp)
ND_PLUGINS=1 ND_PLUGIN_PATH="$SO" \
  ND_ACL_GRANTS='{"defaultWindow":["core:commit","core:window.create"],"grants":[{"window":0,"permissions":["plugin:hello.greet"]}]}' \
  ND_SCRIPT=scripts/m10-plugin-drive.ts timeout 8s ./zig-out/bin/nd-hello >"$PLOG" 2>&1 || true
grep -q "ND_PLUGIN_LOADED name=hello" "$PLOG" || { echo "FAIL: plugin not loaded"; cat "$PLOG"; exit 1; }
grep -q "ND_PLUGIN_COMMAND_OK plugin=hello command=greet" "$PLOG" || { echo "FAIL: plugin command denied/failed"; cat "$PLOG"; exit 1; }
echo "M10_PLUGIN_OK"

# Same plugin, capability withheld -> deny.
DLOG=$(mktemp)
ND_PLUGINS=1 ND_PLUGIN_PATH="$SO" \
  ND_ACL_GRANTS='{"defaultWindow":["core:commit","core:window.create"]}' \
  ND_SCRIPT=scripts/m10-plugin-drive.ts timeout 8s ./zig-out/bin/nd-hello >"$DLOG" 2>&1 || true
grep -q "ND_ACL_DENY permission=plugin:hello.greet" "$DLOG" || { echo "FAIL: plugin cmd not denied without cap"; cat "$DLOG"; exit 1; }
echo "M10_PLUGIN_ACL_DENY_OK"
kill "$WP3" 2>/dev/null || true

echo "headless m10: OK (binary bench + ACL deny + plugin load/deny)"
```

- [ ] **Step 3: `chmod +x` and run the harness locally.**

Run:
```bash
chmod +x scripts/headless-m10.sh
nix develop -c bash -c 'zig build && zig build plugin-hello && bun install --frozen-lockfile && ./scripts/headless-m10.sh'
```
Expected: `M10_BENCH ...`, `M10_ACL_DENY_OK`, `M10_PLUGIN_OK`, `M10_PLUGIN_ACL_DENY_OK`, `headless m10: OK`.

- [ ] **Step 4: Wire CI.** In `.github/workflows/ci.yml`, after the `headless m8` step, add:

```yaml
      - name: bun tests (runtime)
        run: nix develop -c bun test runtime/
      - name: build demo plugin
        run: nix develop -c zig build plugin-hello
      - name: headless m10 (binary + acl + plugins)
        run: nix develop -c ./scripts/headless-m10.sh
```

- [ ] **Step 5: Run the FULL gate + the new legs one more time.**

Run:
```bash
nix develop -c bash -c 'bun tools/codegen.ts \
  && git diff --exit-code -- schema packages/react/src/generated src/generated docs/widgets.md docs/styling.md swift/Sources/NDGen/Widgets.swift \
  && zig build test && zig build && zig build plugin-hello \
  && bun install --frozen-lockfile && bun test runtime/ \
  && ./scripts/headless-smoke.sh && ./scripts/headless-m2.sh && ./scripts/kill9-test.sh \
  && ./scripts/headless-m3.sh && ./scripts/headless-m4.sh && ./scripts/headless-m5b.sh \
  && ./scripts/headless-m5c.sh && ./scripts/headless-m8.sh && ./scripts/headless-m10.sh'
```
Expected: every leg green (all existing gates unchanged + the new m10 legs pass).

- [ ] **Step 6: Update `CLAUDE-activeContext.md`.** Add an `M10 done & green` bullet after the M6b line, capturing: binary NDP fast path (encoder `runtime/ndp-binary.ts` through the outbox, decoder `src/ndp_binary.zig` into `protocol.CommitBatch`→`tree.apply`, negotiated via `HelloAck.encodings=["binary","json"]`, JSON stays default, spec §8 golden vector on both sides); ACL (`src/acl.zig`, gate in `runtime.zig`, default grants `core:commit`+`core:window.create` so demos never break, `plugin:*` default-deny, `ND_ACL_DENY` marker, grants via `nd_set_acl`); native plugins (`include/nd_plugin.h` `nd_plugin_v1`, `src/plugin.zig` dlopen loader gated by `ND_PLUGINS=1`, demo `plugins/hello`, `pluginCommand` NDP frame, capability-checked); benchmark (`scripts/bench-10k.ts`, `headless-m10.sh` gate binary ≤ 2× json); vtable untouched (18 fields, `@sizeOf` assert holds); two new C lifecycle fns `nd_set_acl`/`nd_load_plugin` (no vtable churn); Swift shell wired opt-in + `M10_SWIFT_OK` over ssh. **Owner-visible descopes:** wasmtime/Extism deferred (`docs/superpowers/specs/2026-07-11-wasm-plugin-tier.md`) — WASM tier slots behind the same `nd_plugin_v1` surface as a second loader; "manifest feeds codegen" for plugin TS types is a documented follow-up (plugin commands reachable via untyped `pluginCommand` today); D12 Bun lifecycle-blocking = explicit empty `trustedDependencies` (or deferred if no template pkg). Record any binary-spec deviations found during implementation.

- [ ] **Step 7: Commit (exclude CLAUDE-*.md memory-bank files per repo policy — but activeContext IS a tracked project file here; follow the repo's existing pattern: prior milestones committed activeContext, so include it).**

```bash
git add scripts/headless-m10.sh scripts/m10-plugin-drive.ts .github/workflows/ci.yml CLAUDE-activeContext.md
git commit -m "ci(m10): headless benchmark + acl + plugin legs, activeContext"
```

---

## Owner-visible descopes (surfaced here, not buried)

1. **wasmtime/Extism WASM tier is NOT integrated.** Design-doc + ABI shaping only (Task 13). The `nd_plugin_v1` surface is shaped so the WASM tier is a second loader with identical ACL semantics. Rationale: heavy dependency absent from the nix devshell; deserves its own milestone.
2. **"Manifest feeds codegen" (plugin TS types appear automatically) is a follow-up.** M10's minimum-viable plugin registers a command reachable from JS via the untyped `pluginCommand` NDP frame and denied without its capability — the spec §9 codegen integration is documented as deferred (Task 13).
3. **D12 Bun lifecycle-script blocking** is scoped to an explicit `trustedDependencies: []` in `template/package.json` if trivial, else deferred with a note (Task 12) — Bun already blocks postinstall by default.

---

## Self-Review

**1. Spec coverage (M10 definition + D12 + D4 + §9 + §12):**
- Capability ACL enforcement at NDP dispatch → T6 (model) + T7 (gate in `runtime.zig`) + T10 (`nd_set_acl`). ✓
- `nd_plugin_v1` ABI → T8 (header) + T9 (loader + demo) + T10 (`nd_load_plugin`). ✓
- wasmtime/Extism tier → T13 (descoped design doc, ABI-shaped). ✓ (owner-visible)
- Binary command buffer behind the 10k-node benchmark → T2/T3/T5 (encoder/decoder/enum) + T4/T14 (benchmark gate). ✓ 1:1 with JSON op list, negotiated, JSON default.
- §12 golden-frame tests both sides → T2 (Zig §8 vector) + T3 (TS §8 vector). ✓
- §9 metadata manifest → codegen → documented follow-up (T13). ✓ (honest scope)
- All 18 vtable fields non-null / vtable unchanged → T10 keeps the `@sizeOf` assert; new fns are lifecycle, not vtable. ✓
- Mac shell keeps building → T11 + ssh leg. ✓

**2. Placeholder scan:** every code step carries real code; the one `false` branch in a codegen arm is avoided (no `genSwift`-style stubs here). Benchmark bound is concrete (`≤ 2× json + 5ms`). No TBDs.

**3. Type consistency:** `decodeCommitBatch`→`Decoded.batch: protocol.CommitBatch` consumed by `tree.apply` (T2↔T7). `encodeCommitBatchBinary(batch): Uint8Array` consumed by `sendBinaryFrame` (T3). `WIDGET_TYPE`/`widgetNameOf` from T5 consumed by T2/T3. `Acl.isAllowed(window_id, permission)` consistent across T6/T7/T9/T10. `nd_set_acl`/`nd_load_plugin` signatures match between `include/nd.h` (T10), `src/abi.zig` (T10), and Swift (T11). `nd_plugin_v1` field order identical in `include/nd_plugin.h` (T8), `plugins/hello/plugin.zig` (T8), `src/plugin.zig` (T9). `NdContext.acl`/`NdContext.plugin` added once each (T7/T10). Markers (`ND_ACL_DENY`, `ND_PLUGIN_LOADED`, `ND_PLUGIN_COMMAND_OK`, `ND_BENCH_MOUNT`) match between emitters and `headless-m10.sh` greps.
