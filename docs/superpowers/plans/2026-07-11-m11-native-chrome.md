# M11: Native Chrome Widgets Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make apps built on NativeDesktop look like real GNOME/Adwaita (and, best-effort, real macOS) apps — a `cssClasses` escape hatch onto platform design-system classes, an Adwaita runtime, and two native chrome widgets (`SplitView` sidebar+content, `HeaderBar`), demonstrated by restyling `examples/notes` from "a competent custom skin" into something that reads as native.

**Architecture:** Every widget/prop change flows through the M5 codegen pipeline (`schema/widgets.json` → `tools/codegen.ts` → generated TS/Zig/Swift + docs) — generated files are NEVER hand-edited. New behavior lands in the emitter tables (`STRUCTURAL`/`SWIFT_STRUCTURAL`, the `if/else-if` create/apply body builders) and the hand-written companions (`src/protocol.zig`'s `Attached` struct, `src/gtk/style.zig`, `src/gtk/main.zig`, the Swift `Backend.swift`/`Widgets.swift` dispatch). The Linux embedder switches to libadwaita (`AdwApplication`); AppKit maps chrome to `NSSplitViewController`/`NSVisualEffectView` best-effort. Two small suitability-report repairs (TextArea zero-height floor; automation `getTree` child ordering) ride along.

**Tech Stack:** Zig 0.16, Bun/TypeScript, React 19 reconciler, GTK4 + libadwaita (vendored zig-gobject bindings), Swift 6.2 / AppKit, weston-headless CI.

---

## Global Constraints

- **Zig 0.16 API drift (carry into every Zig task):** no `std.heap.GeneralPurposeAllocator`, no `std.posix.getenv`/`unlink`, no `std.time.milliTimestamp`/`std.Thread.sleep`; sockets live in `std.Io.net`; `std.Io.Mutex` needs `.init` (a bare `self.* = undefined` skips field defaults and bricks it); use `std.Io.sleep(io, .fromMilliseconds(n), .awake)` and poll-count bounds, never wall-clock. Every test-bearing Zig file needs its OWN `addTest` root in `build.zig` — Zig 0.16 does NOT collect tests transitively through `@import`.
- **Generated files are never hand-edited.** The 8 codegen outputs are: `packages/react/src/generated/{intrinsics,schema-meta,widget-types}.ts`, `src/generated/{widgets,widget_types}.zig`, `docs/widgets.md`, `docs/styling.md`, `swift/Sources/NDGen/Widgets.swift`. Every behavior change goes through `tools/codegen.ts` and/or `schema/widgets.json`. CI has a codegen-freshness gate (`bun tools/codegen.ts && git diff --exit-code` on the generated paths).
- **`tools/codegen.ts` is a single-owner file.** Only ONE task per wave may modify it. Same for `schema/widgets.json`, `src/protocol.zig`, `src/tree.zig`, `build.zig`, `src/gtk/main.zig`, `examples/notes/main.tsx`.
- **Commit discipline (index races happened in M9):** every task's commit uses EXPLICIT pathspecs — never `git add -A` or a bare `git add .`. After committing, verify with `git show --stat HEAD` that only the intended files landed.
- **Full gate (run before starting and after the final task):**
  ```
  nix develop -c bash -c 'bun tools/codegen.ts && git diff --exit-code -- schema packages/react/src/generated src/generated docs/widgets.md docs/styling.md swift/Sources/NDGen/Widgets.swift && zig build test && zig build && bun install --frozen-lockfile && ./scripts/headless-smoke.sh && ./scripts/headless-m2.sh && ./scripts/kill9-test.sh && ./scripts/headless-m3.sh && ./scripts/headless-m4.sh && ./scripts/headless-m5b.sh && ./scripts/headless-m5c.sh && ./scripts/headless-m8.sh && ./scripts/headless-m9.sh && ./scripts/headless-m10.sh && ./scripts/headless-notes.sh'
  ```
- **Mac legs go over ssh.** The Mac login shell is FISH — all remote commands via `ssh macbook 'bash -euo pipefail -s' <<'REMOTE' … REMOTE` heredocs. `ssh` prints harmless port-forward bind noise on stderr (filter it, never treat as error). `zig`'s archiver emits `.a` members Apple's `ld` rejects — repack with `ar x` + `chmod 644` + `libtool -static` before `swiftc` links (already in `scripts/mac/mac-run.sh`/`mac-build.sh`). `build.zig`'s `libnd.bundle_compiler_rt = true` is required for the swiftc link.
- **Weston socket uniqueness:** every headless script MUST use a unique `WAYLAND_DISPLAY` name to avoid CI collisions.
- **Markers (`ND_*`) print to stderr; scripts capture `2>&1`.** New markers introduced by this plan: `ND_NAVCHROME_OK` (notes-drive success), plus assertions on existing `ND_NOTES_OK`.

---

## File Structure (decomposition map)

**Wave 0 — research + bindings (blocks everything Adw-dependent):**
- `scripts/regen-bindings.sh` — extend `-Dmodules` to add `Adw-1`.
- `vendor/gobject-bindings/**` — regenerated (adds an `adwaita1` module).
- `build.zig` — add the `adw` import to `gtk_imports`.
- `docs/superpowers/plans/2026-07-11-m11-native-chrome.md` — record the Task-0 decision inline (edit this plan's Task-0 checkbox notes).

**Wave 1 — independent, no Adw dependency (parallel):**
- Task 2 (`cssClasses` schema + TS validator + codegen): `schema/widgets.json`, `tools/codegen.ts`, `packages/react/src/css-classes-validate.ts` (new), `packages/react/src/css-classes-validate.test.ts` (new), `packages/react/src/host-config.ts`.
- Task 3 (automation `getTree` ordering fix): `src/tree.zig`, `src/automation.zig`.
- Task 4 (TextArea zero-height floor): `schema/widgets.json`, `tools/codegen.ts`.

  > NOTE: Tasks 2 and 4 both touch `schema/widgets.json` and `tools/codegen.ts` — they are **serialized within Wave 1** (Task 2 then Task 4, or merged sequentially), NOT run in parallel. Task 3 is fully disjoint (Zig core only) and runs in parallel with them.

**Wave 2 — Adw runtime + cssClasses Zig/Swift sides (depends on Wave 0 + Task 2):**
- Task 5 (Adwaita runtime): `src/gtk/main.zig`, `tools/codegen.ts` (Window→AdwApplicationWindow create arm), `docs/agents/styling.md`.
- Task 6 (`cssClasses` GTK + AppKit appliers): `src/gtk/style.zig`, `swift/Sources/NDShell/Backend.swift`, `tools/codegen.ts` (Swift no-op note only), `docs/styling.md` regen.

  > NOTE: Tasks 5 and 6 both touch `tools/codegen.ts` — serialize them (Task 5 then Task 6).

**Wave 3 — the two chrome widgets (depends on Waves 0–2):**
- Task 7 (`SplitView` widget): `schema/widgets.json`, `src/protocol.zig`, `tools/codegen.ts`, `src/null_backend.zig` (if the null structural path needs a slot branch), `src/conformance.zig`.
- Task 8 (`HeaderBar` widget): `schema/widgets.json`, `src/protocol.zig`, `tools/codegen.ts`, `src/conformance.zig`.

  > NOTE: Tasks 7 and 8 both touch `schema/widgets.json`, `src/protocol.zig`, `tools/codegen.ts`, `src/conformance.zig` — **serialize them** (Task 7 fully done + committed, then Task 8). They are one owner's files.

**Wave 4 — acceptance (depends on everything):**
- Task 9 (restyle notes + drive script + headless leg): `examples/notes/main.tsx`, `scripts/notes-drive.ts`, `scripts/headless-notes.sh`, `.github/workflows/ci.yml`.
- Task 10 (Mac leg + integration): `scripts/mac/mac-m11.sh` (new), `.github/workflows/mac.yml`, `CLAUDE-activeContext.md`, full gate.

---

## Key facts every implementer needs (read before starting)

- **Schema shape:** `schema/widgets.json` is one JSON object: `{ schemaVersion, style, widgets: [...] }`. A widget: `{ name, intrinsic, container: {childModel, attachedProps?} | null, stub?, props: [{name,type,default?,appliesTo,values?}], events, automation: {role, textFrom} }`. `appliesTo` ∈ `create` | `createAndUpdate` | `meta`. `PropType` ∈ `string|int|bool|enum|float|stringList`.
- **The codegen has NO runtime widget table.** `create`/`applyProps` are giant `if (kind==="X"){…} else if …` chains built by `genZigCreateBody`/`genZigApplyBody`/`genSwiftCreateBody`/`genSwiftApplyBody`, each ending in a fail-loud `throw new Error(...)` default. Container structural behavior lives in TWO hand-written `Record<string,Template>` tables: `STRUCTURAL` (Zig, `tools/codegen.ts` ~line 747) and `SWIFT_STRUCTURAL` (Swift, ~line 1217). `genZigStructural`/`genSwiftStructural` THROW at codegen time if a container widget lacks an entry. Event wiring is `SIGNALS`/`SWIFT_SIGNALS` tables keyed `"Widget.eventName"`.
- **Attached props are NOT fully codegen-derived on the Zig side.** `container.attachedProps` in schema drives (a) the TS prop-union typing in `intrinsics.ts` (every intrinsic gets EVERY attached prop, no narrowing) and (b) the Swift `attachedPrelude` decode. But the Zig `protocol.Attached` struct (`src/protocol.zig:53-71`) is **hand-maintained** — adding a new attached prop (e.g. `slot`) requires hand-editing that struct's fields AND its `fromProps` body, mirroring how `tab_label` is handled. Keep them in sync manually.
- **Style is data-driven + cross-cutting.** Every intrinsic gets `style?: StyleProp`. `validateStyle` (`packages/react/src/style-validate.ts`, hand-written) Levenshtein-checks keys against `styleKeySpec` (emitted into `intrinsics.ts`). Zig `src/gtk/style.zig::applyStyle(widget, node_id, std.json.Value)` walks `generated.style_keys`/`style_subkeys`; `addCssClass` is called exactly ONCE per node with the internal `nd-<id>` scoped class (`style.zig:163-175`). AppKit `ndApplyStyle` (`Backend.swift:253-284`) handles the 6 keys via `NSColor`/`NSFont`/frame insets — there is NO CSS-class concept on AppKit.
- **C ABI takes widget kind as `const char*`** (`include/nd.h`) — adding widgets needs ZERO C-header changes. The vtable has 18 fields, all non-null (core calls them unconditionally per commit; a null fn ptr = SIGSEGV).
- **`gtk_grid_attach(grid, child, column, row, width, height)`** — column precedes row (a gotcha the Grid emitter already handles).

---

### Task 0: Research + regenerate bindings with `Adw-1` (Wave 0, BLOCKING)

**Files:**
- Modify: `scripts/regen-bindings.sh:20`
- Modify: `build.zig:25-34` (the `gtk_imports` array)
- Regenerate: `vendor/gobject-bindings/**`
- Modify (record decision): this plan file's Task-0 checkboxes

**Interfaces:**
- Produces: an importable `adw` Zig module (`@import("adw")`) exposing `adw.Application`, `adw.ApplicationWindow`, `adw.OverlaySplitView`, `adw.HeaderBar`, `adw.WindowTitle`, `adw.init` — consumed by Tasks 5, 7, 8.

**Research already done (recorded here so the implementer does not repeat it):**
- `adw` is NOT in the current vendored bindings (14 modules, none Adwaita).
- libadwaita IS in the devshell: `flake.nix:31` lists `libadwaita`; `pkg-config --exists libadwaita-1` returns found; the `Adw-1` GIR is on `XDG_DATA_DIRS`.
- No C-shim precedent exists (no `.c` files, no `extern fn` C-library calls). **Decision: regenerate the vendored bindings to add `Adw-1` — do NOT hand-write a C shim.**
- `scripts/regen-bindings.sh` runs `zig build codegen -Dmodules=Gtk-4.0 "${FLAGS[@]}"` after cloning zig-gobject at pin `97caf8bfb4386409aab1160f7ec05c32ee6d5d7d`. Regen requires NETWORK (git clone) + the devshell.

- [ ] **Step 1: Confirm the Adw-1 GIR is present**

Run: `nix develop -c bash -c 'echo "$XDG_DATA_DIRS" | tr : "\n" | while read p; do ls "$p/gir-1.0/Adw-1.gir" 2>/dev/null; done'`
Expected: prints a path ending in `Adw-1.gir`. **HARD FALLBACK DECISION POINT:** if this prints nothing, the devshell lacks the GIR even though the `.pc` exists. In that case STOP and add `gobject-introspection` + a `libadwaita.dev` (with typelib/GIR) output to `flake.nix`'s Linux inputs, `git add flake.nix`, and re-run this step before proceeding. Do NOT fall back to a C shim — there is no precedent for one and it would be net-new ABI surface.

- [ ] **Step 2: Extend the regen module list**

Edit `scripts/regen-bindings.sh:20`, changing `-Dmodules=Gtk-4.0` to `-Dmodules=Gtk-4.0,Adw-1`:
```bash
(cd "$WORK/zig-gobject" && zig build codegen -Dmodules=Gtk-4.0,Adw-1 "${FLAGS[@]}")
```

- [ ] **Step 3: Regenerate the bindings**

Run: `nix develop -c ./scripts/regen-bindings.sh`
Expected: prints `regenerated vendor/gobject-bindings from zig-gobject@97caf8b...`. Then confirm the new module exists:
Run: `fd -i adw vendor/gobject-bindings`
Expected: a file under `vendor/gobject-bindings/` for the Adwaita module. Note the exact module name zig-gobject assigned (very likely `adwaita1`, matching the `gtk4`/`gsk4` convention where the trailing digit is the GIR major version). Record that exact name in this plan's Task-0 notes for Steps 4–5 and Tasks 5/7/8.

- [ ] **Step 4: Wire the `adw` module into build.zig**

Edit `build.zig:25-34`, appending one line to `gtk_imports` (use the module name confirmed in Step 3, shown here as `adwaita1`):
```zig
    const gtk_imports = [_]std.Build.Module.Import{
        .{ .name = "glib", .module = gobject.module("glib2") },
        .{ .name = "gobject", .module = gobject.module("gobject2") },
        .{ .name = "gio", .module = gobject.module("gio2") },
        .{ .name = "gtk", .module = gobject.module("gtk4") },
        .{ .name = "gsk", .module = gobject.module("gsk4") },
        .{ .name = "gdk", .module = gobject.module("gdk4") },
        .{ .name = "graphene", .module = gobject.module("graphene1") },
        .{ .name = "adw", .module = gobject.module("adwaita1") },
        .{ .name = "build_options", .module = build_options_mod },
    };
```

- [ ] **Step 5: Add a compile-only smoke import to prove the module links**

Add a temporary throwaway file `src/gtk/adw_smoke.zig` that just imports adw and references one symbol, wired via its own `addTest` root in `build.zig` (mirror how `style_test_generated_mod` is wired at `build.zig:137`). Contents:
```zig
const std = @import("std");
const adw = @import("adw");
test "adw module links" {
    // Reference a symbol so the binding is actually compiled/linked.
    const T = adw.Application;
    _ = T;
}
```
Run: `nix develop -c zig build test 2>&1 | tail -5`
Expected: PASS (all tests). If the module name in Step 4 was wrong, this fails with "no module named 'adw'" or a missing-symbol error — go back to Step 3's `fd` output for the real name. Once green, DELETE `src/gtk/adw_smoke.zig` and remove its `addTest` root (it was only to prove linkage).

- [ ] **Step 6: Verify the existing gate still passes with the new bindings**

Run: `nix develop -c bash -c 'zig build test && zig build && ./scripts/headless-smoke.sh'`
Expected: builds clean, smoke passes. Regenerating the bindings must not perturb any existing GTK behavior (the new `adwaita1` module is additive).

- [ ] **Step 7: Commit**

```bash
git add scripts/regen-bindings.sh build.zig vendor/gobject-bindings docs/superpowers/plans/2026-07-11-m11-native-chrome.md
git commit -m "feat(mac): add Adw-1 to vendored gobject bindings + wire adw module"
git show --stat HEAD | grep -qE 'regen-bindings.sh|build.zig|gobject-bindings' || { echo "FAIL: unexpected commit contents"; exit 1; }
```

---

### Task 2: `cssClasses` escape hatch — schema + TS validator + codegen (Wave 1)

**Files:**
- Modify: `schema/widgets.json` (add a top-level `cssClasses` allowlist block)
- Modify: `tools/codegen.ts` (emit `cssClassSpec` into `intrinsics.ts`; add `cssClasses?: string[]` to every intrinsic's JSX field list)
- Create: `packages/react/src/css-classes-validate.ts`
- Test: `packages/react/src/css-classes-validate.test.ts`
- Modify: `packages/react/src/host-config.ts` (call the validator at create + update)

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces: `export function validateCssClasses(classes: unknown): void` (throws `CssClassError` on unknown class); `export const cssClassSpec: string[]` (in `intrinsics.ts`, regenerated); a `cssClasses?: string[]` prop on every JSX intrinsic. Task 6 consumes the wire prop on the Zig/Swift side.

- [ ] **Step 1: Add the allowlist to the schema**

Edit `schema/widgets.json`, adding a top-level `"cssClasses"` key (sibling of `"style"` and `"widgets"`) — the real libadwaita `style-classes.html` list, deprecated classes deliberately excluded (they still work but we steer callers to the current names):
```json
  "cssClasses": [
    "suggested-action", "destructive-action", "flat", "raised", "circular", "pill",
    "linked", "toolbar", "spacer",
    "title-1", "title-2", "title-3", "title-4", "heading", "document", "body",
    "caption-heading", "caption", "monospace", "numeric",
    "accent", "success", "warning", "error",
    "boxed-list", "boxed-list-separate", "card", "activatable",
    "navigation-sidebar", "selection-mode", "osd",
    "dimmed", "background", "view", "frame", "compact", "menu", "inline"
  ],
```

- [ ] **Step 2: Write the failing validator test**

Create `packages/react/src/css-classes-validate.test.ts` (mirror the structure of `style-validate.test.ts`):
```ts
import { test, expect } from "bun:test";
import { validateCssClasses, CssClassError } from "./css-classes-validate.ts";

test("accepts known Adwaita classes", () => {
  expect(() => validateCssClasses(["navigation-sidebar", "pill", "title-2"])).not.toThrow();
});

test("accepts undefined/empty", () => {
  expect(() => validateCssClasses(undefined)).not.toThrow();
  expect(() => validateCssClasses([])).not.toThrow();
});

test("rejects an unknown class with a fix-it hint", () => {
  expect(() => validateCssClasses(["navigation-sidbar"])).toThrow(CssClassError);
  try {
    validateCssClasses(["navigation-sidbar"]);
  } catch (e) {
    expect((e as Error).message).toContain('Did you mean "navigation-sidebar"');
  }
});

test("rejects a totally unknown class by listing valid ones", () => {
  expect(() => validateCssClasses(["flexbox"])).toThrow(CssClassError);
});

test("rejects a non-string array element", () => {
  expect(() => validateCssClasses([42 as unknown as string])).toThrow(CssClassError);
});
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `nix develop -c bun test packages/react/src/css-classes-validate.test.ts`
Expected: FAIL — "Cannot find module './css-classes-validate.ts'".

- [ ] **Step 4: Emit `cssClassSpec` from codegen**

In `tools/codegen.ts`, extend the `Schema` interface with `cssClasses?: string[]`. In `genIntrinsics` (right after the `genStyleProp(s)` call that emits `styleKeySpec`), emit the allowlist as a runtime array:
```ts
function genCssClassSpec(s: Schema): string {
  const classes = s.cssClasses ?? [];
  return `export const cssClassSpec: string[] = ${JSON.stringify(classes)};\n`;
}
```
Call it from `genIntrinsics` alongside `genStyleProp(s)`. Then, in the per-intrinsic JSX field-list builder (where `fields.push("style?: StyleProp")` lives), add `fields.push("cssClasses?: string[]")` so every intrinsic accepts the prop.

- [ ] **Step 5: Regenerate and confirm the spec landed**

Run: `nix develop -c bun tools/codegen.ts && rg "cssClassSpec" packages/react/src/generated/intrinsics.ts`
Expected: `export const cssClassSpec: string[] = ["suggested-action",...];`. Also confirm `cssClasses?: string[]` appears on an intrinsic: `rg -c "cssClasses\?: string\[\]" packages/react/src/generated/intrinsics.ts` (should be > 1, one per widget).

- [ ] **Step 6: Write the validator (mirror `style-validate.ts` exactly)**

Create `packages/react/src/css-classes-validate.ts`:
```ts
import { cssClassSpec } from "./generated/intrinsics.ts";

export class CssClassError extends Error {}

function nearest(bad: string, valid: string[]): string | null {
  let best: string | null = null, bestD = Infinity;
  for (const v of valid) {
    const d = lev(bad, v);
    if (d < bestD) { bestD = d; best = v; }
  }
  return bestD <= 3 ? best : null;
}

function lev(a: string, b: string): number {
  const m = a.length, n = b.length;
  const dp: number[][] = Array.from({ length: m + 1 }, (_, i) => [i, ...Array(n).fill(0)]);
  for (let j = 0; j <= n; j++) dp[0]![j] = j;
  for (let i = 1; i <= m; i++) for (let j = 1; j <= n; j++)
    dp[i]![j] = Math.min(dp[i - 1]![j]! + 1, dp[i]![j - 1]! + 1, dp[i - 1]![j - 1]! + (a[i - 1] === b[j - 1] ? 0 : 1));
  return dp[m]![n]!;
}

export function validateCssClasses(classes: unknown): void {
  if (classes == null) return;
  if (!Array.isArray(classes)) throw new CssClassError(`cssClasses must be a string[] — got ${typeof classes}. See docs/styling.md`);
  for (const c of classes) {
    if (typeof c !== "string") throw new CssClassError(`cssClasses entries must be strings — got ${typeof c}. See docs/styling.md`);
    if (!cssClassSpec.includes(c)) {
      const near = nearest(c, cssClassSpec);
      const hint = near ? `Did you mean "${near}"?` : `Valid classes: ${cssClassSpec.join(", ")}.`;
      throw new CssClassError(`Unknown CSS class "${c}" — only Adwaita/GTK design-system classes are allowed. ${hint} See docs/styling.md`);
    }
  }
}
```

- [ ] **Step 7: Run the test to verify it passes**

Run: `nix develop -c bun test packages/react/src/css-classes-validate.test.ts`
Expected: PASS (5 tests).

- [ ] **Step 8: Wire the validator into host-config**

In `packages/react/src/host-config.ts`, the create site is `host-config.ts:55` (`if ("style" in props) validateStyle(props.style);`) and the update site is `host-config.ts:129` (`if ("style" in newProps) validateStyle(newProps.style);`). `props`/`newProps` is a spread copy of `inst.props`, so `cssClasses` flows through the props object untouched. Import the validator alongside the existing `import { validateStyle } from "./style-validate.ts";`:
```ts
import { validateCssClasses } from "./css-classes-validate.ts";
```
Add a parallel guard immediately after each `validateStyle` call — at line 55 (using `props`) and line 129 (using `newProps`):
```ts
if ("cssClasses" in props) validateCssClasses(props.cssClasses);
```
(and the `newProps` variant at the update site).

- [ ] **Step 9: Add a test-suite freshness run**

Run: `nix develop -c bash -c 'bun test packages/react/src/css-classes-validate.test.ts && bun test packages/react/src/style-validate.test.ts'`
Expected: both PASS. (The style test must still be green — no regression to the existing validator.)

- [ ] **Step 10: Commit**

```bash
git add schema/widgets.json tools/codegen.ts packages/react/src/generated packages/react/src/css-classes-validate.ts packages/react/src/css-classes-validate.test.ts packages/react/src/host-config.ts docs/widgets.md docs/styling.md
git commit -m "feat(react): cssClasses escape hatch validated against the Adwaita class allowlist"
git show --stat HEAD | grep -q css-classes-validate.ts || { echo "FAIL: validator not committed"; exit 1; }
```

---

### Task 3: Automation `getTree` child ordering follows real sibling order (Wave 1, disjoint)

**Files:**
- Modify: `src/tree.zig` (add an ordered per-parent children list to the tree state, maintained by `apply`'s append/insertBefore/remove op handlers)
- Modify: `src/automation.zig` (`handleGetTree` consumes the ordered list instead of grouping over a hashmap scan)
- Test: `src/tree.zig`'s own `test` block (already has an `addTest` root)

**Interfaces:**
- Consumes: nothing.
- Produces: `pub fn childrenOf(self: *Tree, id: u32) []const u32` on `Tree` — returns the ordered child ids of a node (empty slice if none). Consumed by `handleGetTree`.

**Root cause (confirmed):** `handleGetTree` (`src/automation.zig:323-368`) builds `children_of` by iterating `tree.meta` — an `AutoHashMapUnmanaged`, whose iteration order is hash-bucket layout, NOT insertion order (the "insertion order" comment at that call site is false). `NodeMeta` (`src/tree.zig:11-20`) stores only `parent: u32` — there is NO ordered sibling list anywhere. Sorting by id is unsound because `insertBefore` reorders without changing ids. The fix is a real ordered children list in `Tree`, maintained in the op handlers.

- [ ] **Step 1: Write the failing ordering test**

Add to `src/tree.zig`'s test block (near the existing tests). This exercises the meta/children model without a real embedder (use `Tree.initBare`, which skips the app handle):
```zig
test "childrenOf preserves append + insertBefore order" {
    const gpa = std.testing.allocator;
    var tree = Tree.initBare(gpa);
    defer tree.deinitMeta();
    defer tree.deinitChildren(); // new: frees the ordered children lists

    // Parent p=1, children appended 10, 20, 30.
    try tree.putMeta(1, "Box", null, null, 0, .{});
    try tree.putMeta(10, "Label", null, null, 0, .{});
    try tree.putMeta(20, "Label", null, null, 0, .{});
    try tree.putMeta(30, "Label", null, null, 0, .{});
    tree.recordAppend(1, 10);
    tree.recordAppend(1, 20);
    tree.recordAppend(1, 30);
    try std.testing.expectEqualSlices(u32, &.{ 10, 20, 30 }, tree.childrenOf(1));

    // insertBefore 30 -> new child 25 lands between 20 and 30.
    try tree.putMeta(25, "Label", null, null, 0, .{});
    tree.recordInsertBefore(1, 25, 30);
    try std.testing.expectEqualSlices(u32, &.{ 10, 20, 25, 30 }, tree.childrenOf(1));

    // remove 20 -> order compacts.
    tree.recordRemove(1, 20);
    try std.testing.expectEqualSlices(u32, &.{ 10, 25, 30 }, tree.childrenOf(1));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `nix develop -c zig build test 2>&1 | tail -20`
Expected: FAIL — `recordAppend`/`childrenOf`/`deinitChildren`/`recordInsertBefore`/`recordRemove` are undefined.

- [ ] **Step 3: Add the ordered children map to Tree**

In `src/tree.zig`, add a field to `Tree` (near `meta`):
```zig
    children: std.AutoHashMapUnmanaged(u32, std.ArrayList(u32)) = .{},
```
Add these methods to `Tree` (place them near `setMetaParent`):
```zig
    pub fn recordAppend(self: *Tree, parent: u32, child: u32) void {
        self.detachFromParent(child); // moving an already-mounted child re-homes it
        const gop = self.children.getOrPut(self.gpa, parent) catch return;
        if (!gop.found_existing) gop.value_ptr.* = .empty;
        gop.value_ptr.append(self.gpa, child) catch {};
    }

    pub fn recordInsertBefore(self: *Tree, parent: u32, child: u32, before: u32) void {
        self.detachFromParent(child);
        const gop = self.children.getOrPut(self.gpa, parent) catch return;
        if (!gop.found_existing) gop.value_ptr.* = .empty;
        const list = gop.value_ptr;
        var idx: usize = list.items.len;
        for (list.items, 0..) |c, i| if (c == before) { idx = i; break; };
        list.insert(self.gpa, idx, child) catch {};
    }

    pub fn recordRemove(self: *Tree, parent: u32, child: u32) void {
        const list = self.children.getPtr(parent) orelse return;
        for (list.items, 0..) |c, i| if (c == child) { _ = list.orderedRemove(i); return; };
    }

    fn detachFromParent(self: *Tree, child: u32) void {
        // The child's current parent is tracked in meta; drop it from that list.
        const m = self.meta.getPtr(child) orelse return;
        if (m.parent == 0) return;
        self.recordRemove(m.parent, child);
    }

    pub fn childrenOf(self: *Tree, id: u32) []const u32 {
        const list = self.children.getPtr(id) orelse return &.{};
        return list.items;
    }

    pub fn deinitChildren(self: *Tree) void {
        var it = self.children.iterator();
        while (it.next()) |entry| entry.value_ptr.deinit(self.gpa);
        self.children.deinit(self.gpa);
    }
```
NOTE: `recordAppend`/`recordInsertBefore` call `detachFromParent` BEFORE `setMetaParent` runs in the op handler, so `detachFromParent` reads the OLD parent from meta — this is why the op-handler call order in Step 4 is `recordAppend(...)` then `setMetaParent(...)`.

- [ ] **Step 4: Maintain the list from the apply op handlers**

In `src/tree.zig`'s `apply`, in the `append` arm (currently `backend.appendChild(...); self.setMetaParent(op.child.?, op.parent.?);` at lines 203-204), add the record BEFORE `setMetaParent`:
```zig
                backend.appendChild(parent_widget, pmeta.widget_type, child_widget, cmeta.attached);
                self.recordAppend(op.parent.?, op.child.?);
                self.setMetaParent(op.child.?, op.parent.?);
```
In the `insertBefore` arm (lines 230-231):
```zig
                backend.insertBefore(parent_widget, pmeta.widget_type, child_widget, before, cmeta.attached);
                if (op.before) |b| self.recordInsertBefore(op.parent.?, op.child.?, b) else self.recordAppend(op.parent.?, op.child.?);
                self.setMetaParent(op.child.?, op.parent.?);
```
In the `remove` arm (lines 232-249), before `self.removeMeta(op.id.?)`:
```zig
                if (self.metaGet(op.id.?)) |cmeta| self.recordRemove(cmeta.parent, op.id.?);
                _ = self.nodes.remove(op.id.?);
                self.removeMeta(op.id.?);
```
Wire `deinitChildren` into wherever `deinitMeta` is called at teardown (search for `deinitMeta` call sites and add `deinitChildren` alongside).

- [ ] **Step 5: Run the tree test to verify it passes**

Run: `nix develop -c zig build test 2>&1 | tail -20`
Expected: PASS. Fix the FIRST compile error only if any (later errors usually cascade).

- [ ] **Step 6: Rewrite handleGetTree to consume the ordered list**

In `src/automation.zig`'s `handleGetTree` (lines 323-368), DELETE the `children_of` hashmap-scan build loop (and its stale "insertion order" comment). Replace `buildNode`'s child-sequence source: instead of `children_of.get(id)`, call `tree.childrenOf(id)` directly. The overlay-node sentinel (`parent == 0` attaches under root) still needs handling — but now that append/insertBefore record into the ordered list, overlay nodes registered via `registerOverlayNode` must ALSO call `tree.recordAppend(root_id, overlay_id)` (check `src/overlay.zig`'s `registerOverlayNode`; if it only sets meta parent, add the `recordAppend` there so the overlay still surfaces in getTree). Read `handleGetTree` and `buildNode` fully before editing; keep the arena-based node allocation intact.

- [ ] **Step 7: Verify automation ordering end to end**

Run: `nix develop -c bash -c 'zig build && ./scripts/headless-m4.sh && ./scripts/headless-m5b.sh'`
Expected: both green (they exercise getTree over real trees). The m5b gallery drive walks nested children — its assertions must still pass with real sibling ordering.

- [ ] **Step 8: Commit**

```bash
git add src/tree.zig src/automation.zig src/overlay.zig
git commit -m "fix(automation): getTree child ordering follows real sibling order, not hashmap iteration"
git show --stat HEAD | grep -q src/tree.zig || { echo "FAIL: tree.zig not committed"; exit 1; }
```

---

### Task 4: TextArea zero-height floor (Wave 1, after Task 2's schema edit)

**Files:**
- Modify: `schema/widgets.json` (add `minContentHeight` prop to TextArea)
- Modify: `tools/codegen.ts` (TextArea create arm sets a size request on both backends)

**Interfaces:**
- Consumes: nothing.
- Produces: a `minContentHeight?: number` prop on `<textarea>` with a sane default so an empty TextArea no longer collapses to `h:0` and stays automation-actionable.

**Rationale:** A freshly-created empty TextArea reported `geometry: {w:168, h:0}` in getTree and `setValue` correctly rejected it as not-actionable. The app-level workaround (wrapping every `<textarea>` in `<scrollview minContentHeight={...}>`) should not be required. Fix TextArea's own floor symmetrically on both backends, mirroring how ScrollView's `minContentHeight` already works.

- [ ] **Step 1: Add the prop to schema**

Edit the TextArea block in `schema/widgets.json` (currently props are `text` + `testID`), adding `minContentHeight` with a non-zero default:
```json
    {
      "name": "TextArea",
      "intrinsic": "textarea",
      "container": null,
      "props": [
        { "name": "text", "type": "string", "default": "", "appliesTo": "createAndUpdate" },
        { "name": "minContentHeight", "type": "int", "default": 120, "appliesTo": "create" },
        { "name": "testID", "type": "string", "appliesTo": "meta" }
      ],
      "events": [
        { "name": "changed", "ndpName": "onChanged", "payload": "text" }
      ],
      "automation": { "role": "textbox", "textFrom": "text" }
    },
```

- [ ] **Step 2: Emit the size request in the GTK create arm**

In `tools/codegen.ts`'s `genZigCreateBody` (the `if (w.name === "TextArea")` arm that currently emits the `gtk.TextView.new()` + buffer setup), add a `setSizeRequest` after the buffer text is set. The generated Zig should read:
```zig
    } else if (std.mem.eql(u8, kind, "TextArea")) {
        const view = gtk.TextView.new();
        const buf = gtk.TextView.getBuffer(view);
        if (propStr(props, "text")) |t| { if (t.len > 0) gtk.TextBuffer.setText(buf, dupeZ(t), -1); }
        if (propInt(props, "minContentHeight")) |h| { if (h > 0) gtk.Widget.setSizeRequest(view.as(gtk.Widget), -1, @intCast(h)); }
        return view.as(gtk.Widget);
```
(Mirror the exact string-building style of the surrounding arms in `genZigCreateBody`.)

- [ ] **Step 3: Emit the size request in the Swift create arm**

In `tools/codegen.ts`'s `genSwiftCreateBody`, the TextArea arm wraps an `NSTextView` in an `NSScrollView`. After that, set a minimum height on the scroll view, mirroring ScrollView's `minContentHeight` handling (`sv.frame.size.height = CGFloat(minH)`):
```swift
    } else if kind == "TextArea" {
        // ... existing NSTextView-in-NSScrollView setup ...
        let minH = propInt(props, "minContentHeight") ?? 0
        if minH > 0 { scroll.frame.size.height = CGFloat(minH) }
        return scroll
```
Read the existing TextArea Swift arm first to reference its actual local variable name for the scroll view.

- [ ] **Step 4: Regenerate and confirm both backends set the request**

Run: `nix develop -c bash -c 'bun tools/codegen.ts && rg -n "minContentHeight" src/generated/widgets.zig swift/Sources/NDGen/Widgets.swift'`
Expected: the TextArea arm in `widgets.zig` shows `setSizeRequest(...)`; the Swift arm shows `minH`. Also confirm the docs regenerated: `rg -A2 "TextArea" docs/widgets.md | rg minContentHeight`.

- [ ] **Step 5: Verify build + existing gallery/notes legs**

Run: `nix develop -c bash -c 'zig build test && zig build && ./scripts/headless-notes.sh'`
Expected: green. The notes app currently wraps its TextArea in a ScrollView; that still works (the size request is additive). Task 9 will remove the wrapper.

- [ ] **Step 6: Commit**

```bash
git add schema/widgets.json tools/codegen.ts src/generated/widgets.zig swift/Sources/NDGen/Widgets.swift docs/widgets.md packages/react/src/generated
git commit -m "fix(schema): TextArea gets a minContentHeight floor so empty editors stay actionable"
git show --stat HEAD | grep -q schema/widgets.json || { echo "FAIL: schema not committed"; exit 1; }
```

---

### Task 5: Adwaita runtime — AdwApplication + adw.init + dark mode (Wave 2, depends on Task 0)

**Files:**
- Modify: `src/gtk/main.zig` (swap `gtk.Application.new` → `adw.Application.new`)
- Modify: `tools/codegen.ts` (Window create arm: `gtk.ApplicationWindow.new` → `adw.ApplicationWindow.new`)
- Modify: `docs/agents/styling.md` (document dark-mode interaction with the style prop)

**Interfaces:**
- Consumes: the `adw` module from Task 0.
- Produces: a running Adwaita host — the Adwaita stylesheet is loaded (so Task 6's cssClasses actually render as designed) and the app follows the system `prefer-dark` setting.

**Facts:** `AdwApplication` calls `adw_init()` in its default `startup` handler (which chains up to `GtkApplication`), so switching the application type is the whole job — no separate `adw.init()` call needed. `src/gtk/main.zig:26` is `var app = gtk.Application.new(app_id, .{});`; the window is created in `src/generated/widgets.zig`'s Window arm via `gtk.ApplicationWindow.new(app)` (from the codegen `genZigCreateBody` Window arm). `AdwApplicationWindow` is the Adwaita window that hosts `AdwOverlaySplitView`/`AdwHeaderBar` correctly (its content area is bare, unlike GtkApplicationWindow's implicit box).

**Dark-mode honesty (document, do not paper over):** With Adwaita loaded, unstyled widgets follow the system light/dark preference for free. BUT the existing `style` prop sets literal hardcoded colors (e.g. notes' `background:"#fafafa"`). Those hardcoded colors do NOT adapt to dark mode — they will render light-on-light or light-on-dark exactly as written. This is expected and correct: `style` is an explicit override. The path to dark-mode-correct apps is to drop hardcoded colors and lean on cssClasses + Adwaita defaults (which Task 9's restyle does). Document this interaction in `docs/agents/styling.md`.

- [ ] **Step 1: Swap the application type in main.zig**

Edit `src/gtk/main.zig`. Add `const adw = @import("adw");` to the imports (near the `gtk`/`gio` imports). Change line 26:
```zig
    var app = adw.Application.new(app_id, .{});
```
`adw.Application` IS a `gtk.Application` subclass, so `app.as(gio.Application)` and the existing `gio.Application.signals.activate.connect(app, …)` / `gio.Application.run(...)` calls are unchanged (adw.Application exposes the same GApplication interface). Verify the `--smoke` path (lines 39-49) still compiles — it uses `app` the same way.

- [ ] **Step 2: Verify the host still builds and boots**

Run: `nix develop -c bash -c 'zig build && ./scripts/headless-smoke.sh'`
Expected: build clean, smoke passes. If `adw.Application.new`'s signature differs from `gtk.Application.new` (arg count/types), fix to match the binding — check `vendor/gobject-bindings`'s adwaita module for the exact `Application.new` signature. Fix only the FIRST error.

- [ ] **Step 3: Switch the Window create arm to AdwApplicationWindow**

In `tools/codegen.ts`'s `genZigCreateBody`, the Window arm currently emits `gtk.ApplicationWindow.new(@ptrCast(app))` (or similar — read it first). Change it to `adw.ApplicationWindow.new(...)`. The generated `src/generated/widgets.zig` must import `adw` — check the codegen import-header block (`ZIG` import lines emitted by `genZig`) and add `const adw = @import("adw");` there if the Window arm now references adw. `AdwApplicationWindow`'s content is set via `adw.ApplicationWindow.setContent(win, child)` NOT `gtk.Window.setChild` — so the Window entry in the `STRUCTURAL` table (its `append`/`remove` closures) must also change from `gtk.Window.setChild(...)` to `adw.ApplicationWindow.setContent(...)`. Update both the create arm AND the `STRUCTURAL.Window` entry in the same edit.

- [ ] **Step 4: Regenerate + confirm the Window arm changed**

Run: `nix develop -c bash -c 'bun tools/codegen.ts && rg -n "AdwApplicationWindow|adw.ApplicationWindow|adw\." src/generated/widgets.zig'`
Expected: the Window create arm and its structural append/remove reference `adw.ApplicationWindow`.

- [ ] **Step 5: Verify all existing GTK legs still pass with the Adwaita window**

Run: `nix develop -c bash -c 'zig build test && zig build && ./scripts/headless-m3.sh && ./scripts/headless-m5b.sh && ./scripts/headless-m5c.sh && ./scripts/headless-notes.sh'`
Expected: all green. `AdwApplicationWindow.setContent` replaces `GtkWindow.setChild` for the single-child Window container — the counter/gallery/notes apps all mount one root child, so behavior is equivalent. If any app relied on `GtkApplicationWindow`'s implicit box, it would show here — fix by ensuring the app mounts exactly one root child (they all do).

- [ ] **Step 6: Document the dark-mode interaction**

In `docs/agents/styling.md`, add a section "Adwaita runtime & dark mode" stating: the host runs as `AdwApplication` so unstyled widgets and cssClasses follow the system light/dark preference automatically; hardcoded `style` colors are explicit overrides that do NOT adapt; prefer cssClasses + Adwaita defaults over hardcoded palettes for theme-correct apps.

- [ ] **Step 7: Commit**

```bash
git add src/gtk/main.zig tools/codegen.ts src/generated/widgets.zig docs/agents/styling.md packages/react/src/generated docs/widgets.md docs/styling.md
git commit -m "feat(mac): run the Linux host as AdwApplication for the Adwaita stylesheet + dark mode"
git show --stat HEAD | grep -q src/gtk/main.zig || { echo "FAIL"; exit 1; }
```

---

### Task 6: `cssClasses` appliers — GTK (real) + AppKit (no-op) (Wave 2, depends on Tasks 2, 5)

**Files:**
- Modify: `src/gtk/style.zig` (recognize `cssClasses` and call `gtk.Widget.addCssClass` per class)
- Modify: `swift/Sources/NDShell/Backend.swift` (document + no-op the `cssClasses` key in `ndApplyStyle`)
- Modify: `tools/codegen.ts` ONLY if the wire path needs it — see Step 1 (likely no change; `cssClasses` rides the existing style/props channel)

**Interfaces:**
- Consumes: the validated `cssClasses` prop (Task 2) — arrives on the wire.
- Produces: on GTK, each allowlisted class is added via `gtk.Widget.addCssClass` (additive alongside the internal `nd-<id>` class). On AppKit, a documented no-op.

**Wire-path decision (Step 1 must resolve first):** `cssClasses` is a top-level prop, not nested under `style`. Determine where it arrives: is it in the create/update `props` object (like `orientation`), or does the React host bundle it into `style`? Read `packages/react/src/host-config.ts` to see how `cssClasses` is placed on the NDP op. If it rides in `props`, the applier must be invoked from the props path (`backend.applyProps` / an explicit call in `src/tree.zig`'s create+update arms, mirroring `applyStyleIfPresent`). Prefer a dedicated `applyCssClassesIfPresent(widget, props)` in `src/tree.zig` (like `applyStyleIfPresent` at `tree.zig:47-51`), calling a new `backend.applyCssClasses`. This keeps it out of the style CSS-compile path entirely (cssClasses are NOT CSS text — they are class names).

- [ ] **Step 1: Trace where cssClasses arrives on the wire**

Run: `nix develop -c rg -n "cssClasses" packages/react/src/host-config.ts packages/react/src/`
Read the result. Confirm whether `cssClasses` lands in the op's `props` object. Record the answer; it determines whether Step 3 hooks into `applyProps` or a dedicated tree-level applier. (Expected: it rides in `props` like any other prop, since Task 2 only added it to the JSX field list and validated it — it was not bundled into `style`.)

- [ ] **Step 2: Add the backend vtable-adjacent applier signature**

The C ABI vtable is frozen at 18 fields (do NOT add a vtable field). Instead, apply cssClasses inside the EXISTING `apply_style` path OR as a dedicated tree-level call using the existing `apply_props`. Simplest: handle it in `src/gtk/style.zig` by having `src/tree.zig` route `props.cssClasses` to a new exported `style.applyCssClasses(widget, node_id, json_array)`. Add to `src/tree.zig` a helper mirroring `applyStyleIfPresent`:
```zig
fn applyCssClassesIfPresent(widget: *Widget, id: u32, props: ?std.json.Value) void {
    const v = props orelse return;
    if (v != .object) return;
    if (v.object.get("cssClasses")) |cls| backend.applyCssClasses(widget, id, cls);
}
```
Call it right after `applyStyleIfPresent(...)` in BOTH the create arm (`tree.zig:197`) and the update arm (`tree.zig:218`). `backend.applyCssClasses` is a new function on the backend seam — add it to the backend interface (`src/backend.zig`), the null backend (`src/null_backend.zig`, a no-op), and the GTK backend (`src/gtk/backend.zig`, forwarding to `style.applyCssClasses`). It takes `(widget, node_id, std.json.Value)` — this is a CORE→backend call through the seam, NOT a C-ABI vtable field, so no `include/nd.h` change. (Note: on the Mac path, `src/gtk/backend.zig`'s equivalent is the Swift side — see Step 5. The seam function exists on all backends so the core call is uniform.)

- [ ] **Step 3: Implement the GTK applier in style.zig**

Add to `src/gtk/style.zig`:
```zig
pub fn applyCssClasses(widget: *gtk.Widget, node_id: u32, value: std.json.Value) void {
    _ = node_id;
    if (value != .array) return;
    for (value.array.items) |item| {
        if (item != .string) continue;
        const cls = std.fmt.allocPrintZ(gpa, "{s}", .{item.string}) catch continue;
        defer gpa.free(cls);
        gtk.Widget.addCssClass(widget, cls);
    }
}
```
NOTE: this is additive — it never removes classes. Since apps rarely toggle Adwaita classes at runtime and the validator gates inputs, additive-only is acceptable for M11 (document it). The internal `nd-<id>` class (added once in `applyStyle`) is untouched.

- [ ] **Step 4: Verify GTK adds the classes**

Add a `test` block to `src/gtk/style.zig` (it already has an `addTest` root) OR verify via a headless run in Task 9. For now:
Run: `nix develop -c bash -c 'zig build test && zig build'`
Expected: build clean, tests pass.

- [ ] **Step 5: No-op cssClasses on AppKit (documented)**

In `swift/Sources/NDShell/Backend.swift`, the `apply_style` vtable slot calls `ndApplyStyle`. But cssClasses now rides in `props`, applied via the seam. On the Mac, the seam function's equivalent is reached through the same `apply_props`/tree path — but AppKit has no CSS classes. Add to `Backend.swift` a documented no-op: wherever the Mac decodes props, if `cssClasses` is present, do nothing but leave a comment: `// cssClasses: AppKit has no CSS-class concept — no-op (M11). Native chrome fidelity comes from the SplitView/HeaderBar widgets, not class strings.` If the Mac path routes cssClasses through the same core seam as GTK, the Swift backend needs an `applyCssClasses` entry point too — but since the vtable is frozen and cssClasses is core→backend, confirm how the Mac backend receives per-node core calls. If the Mac cannot receive this call without a vtable change, then cssClasses is simply dropped on the Mac (the props object still carries it, the Swift `ndApplyProps` ignores unknown keys) — document that as the chosen behavior. Do NOT add a vtable field.

- [ ] **Step 6: Document per-class AppKit mapping**

In `docs/styling.md` (regenerate note) or `docs/agents/styling.md`, add a table stating cssClasses is GTK-only; on AppKit it is a no-op, and native macOS look comes from the SplitView/HeaderBar widgets (NSVisualEffectView sidebars) rather than class strings.

- [ ] **Step 7: Verify the full Linux gate legs affected**

Run: `nix develop -c bash -c 'zig build test && zig build && ./scripts/headless-notes.sh && ./scripts/headless-m5c.sh'`
Expected: green.

- [ ] **Step 8: Commit**

```bash
git add src/gtk/style.zig src/backend.zig src/null_backend.zig src/gtk/backend.zig swift/Sources/NDShell/Backend.swift src/tree.zig docs/agents/styling.md docs/styling.md
git commit -m "feat(mac): apply cssClasses via gtk.Widget.addCssClass (GTK); documented no-op on AppKit"
git show --stat HEAD | grep -q src/gtk/style.zig || { echo "FAIL"; exit 1; }
```

---

### Task 7: `SplitView` widget (sidebar + content) (Wave 3, depends on Waves 0–2)

**Files:**
- Modify: `schema/widgets.json` (add SplitView with a `slot` attached prop)
- Modify: `src/protocol.zig` (add `slot` field to `Attached` + `fromProps`)
- Modify: `tools/codegen.ts` (create arm + `STRUCTURAL`/`SWIFT_STRUCTURAL` entries)
- Modify: `src/conformance.zig` (schema-driven test coverage — verify it picks SplitView up; add explicit slot assertions if needed)

**Interfaces:**
- Consumes: `adw` module (Task 0), Adwaita runtime (Task 5).
- Produces: `<splitview>` intrinsic with props `sidebarWidth?` (fraction), `collapsed?`, and a `slot="sidebar"|"content"` attached prop on its two children.

**Widget choice (decided):** Use **`AdwOverlaySplitView`**, NOT `AdwNavigationSplitView`. `AdwNavigationSplitView` requires each child wrapped in an `AdwNavigationPage` (an extra widget layer that breaks the "child is a plain widget" model). `AdwOverlaySplitView` accepts plain `GtkWidget` via `adw_overlay_split_view_set_sidebar(self, GtkWidget*)` / `set_content(self, GtkWidget*)` — a direct fit for a persistent two-pane sidebar. **Fallback if the adw binding proves infeasible:** `GtkPaned` (orientation horizontal) with the sidebar child given the `.navigation-sidebar` class — but Task 0 proved the adw binding exists, so `AdwOverlaySplitView` is the path.

**Slot mechanism (mirror TabView's tabLabel exactly):** children carry `slot="sidebar"` or `slot="content"`; the `STRUCTURAL.SplitView.append` closure branches on `attached.slot` (string compare, exactly like TabView branches on `attached.tab_label`) to call `set_sidebar` vs `set_content`.

- [ ] **Step 1: Add SplitView to schema**

Edit `schema/widgets.json`, adding a widget entry (mirror TabView's `container.attachedProps` shape):
```json
    {
      "name": "SplitView",
      "intrinsic": "splitview",
      "container": {
        "childModel": "multi",
        "attachedProps": [
          { "name": "slot", "type": "enum", "values": ["sidebar", "content"], "default": "content" }
        ]
      },
      "props": [
        { "name": "sidebarWidth", "type": "float", "default": 0.0, "appliesTo": "create" },
        { "name": "collapsed", "type": "bool", "default": false, "appliesTo": "createAndUpdate" },
        { "name": "testID", "type": "string", "appliesTo": "meta" }
      ],
      "events": [],
      "automation": { "role": "group", "textFrom": null }
    },
```

- [ ] **Step 2: Add the `slot` field to protocol.Attached (hand-edit)**

Edit `src/protocol.zig`'s `Attached` struct (lines 53-71). Add the field and its `fromProps` decode, mirroring `tab_label`:
```zig
pub const Attached = struct {
    grid_row: i64 = 0,
    grid_column: i64 = 0,
    grid_row_span: i64 = 1,
    grid_column_span: i64 = 1,
    tab_label: ?[]const u8 = null,
    slot: ?[]const u8 = null,

    pub fn fromProps(props: ?std.json.Value) Attached {
        var a = Attached{};
        const v = props orelse return a;
        if (v != .object) return a;
        if (v.object.get("gridRow")) |f| { if (f == .integer) a.grid_row = f.integer; }
        if (v.object.get("gridColumn")) |f| { if (f == .integer) a.grid_column = f.integer; }
        if (v.object.get("gridRowSpan")) |f| { if (f == .integer) a.grid_row_span = f.integer; }
        if (v.object.get("gridColumnSpan")) |f| { if (f == .integer) a.grid_column_span = f.integer; }
        if (v.object.get("tabLabel")) |f| { if (f == .string) a.tab_label = f.string; }
        if (v.object.get("slot")) |f| { if (f == .string) a.slot = f.string; }
        return a;
    }
};
```
NOTE: `slot` is a borrowed slice here (like `tab_label`). `tree.zig`'s `putMeta`/`removeMeta`/`deinitMeta` dupe/free `tab_label` — check whether `slot` also needs ownership there. Since `slot` is only read at attach time (never stored past the structural call, unlike `tab_label` which getTree may reference), a borrowed slice that lives only during `apply` is sufficient — but confirm `Attached` copies stored in `NodeMeta` don't outlive the op's JSON. If `NodeMeta.attached` retains `slot`, dupe/free it in `putMeta`/`removeMeta`/`deinitMeta` exactly as `tab_label` is handled (tree.zig:118, 154, 163).

- [ ] **Step 3: Add the create arm + STRUCTURAL entry in codegen (Zig)**

In `tools/codegen.ts`:
1. `genZigCreateBody` — add a SplitView arm:
```zig
    } else if (std.mem.eql(u8, kind, "SplitView")) {
        const sv = adw.OverlaySplitView.new();
        if (propFloat(props, "sidebarWidth")) |w| { if (w > 0) adw.OverlaySplitView.setSidebarWidthFraction(sv, w); }
        if (propBool(props, "collapsed")) |c| adw.OverlaySplitView.setCollapsed(sv, if (c) 1 else 0);
        return sv.as(gtk.Widget);
```
2. `genZigApplyBody` — SplitView has a `createAndUpdate` prop (`collapsed`), so it needs an apply arm:
```zig
    } else if (std.mem.eql(u8, kind, "SplitView")) {
        if (propBool(props, "collapsed")) |c| adw.OverlaySplitView.setCollapsed(@ptrCast(@alignCast(widget)), if (c) 1 else 0);
```
3. `STRUCTURAL` table — add a SplitView entry branching on `attached.slot`:
```ts
  SplitView: {
    append: () => {
      let s = "        const sv: *adw.OverlaySplitView = @ptrCast(@alignCast(parent));\n";
      s += "        if (attached.slot) |sl| {\n";
      s += "            if (std.mem.eql(u8, sl, \"sidebar\")) adw.OverlaySplitView.setSidebar(sv, child)\n";
      s += "            else adw.OverlaySplitView.setContent(sv, child);\n";
      s += "        } else adw.OverlaySplitView.setContent(sv, child);\n";
      return s;
    },
    insertBefore: () => {
      // Two named slots, not an ordered list — insertBefore is set-by-slot, same as append.
      let s = "        _ = before;\n";
      s += "        const sv: *adw.OverlaySplitView = @ptrCast(@alignCast(parent));\n";
      s += "        if (attached.slot) |sl| {\n";
      s += "            if (std.mem.eql(u8, sl, \"sidebar\")) adw.OverlaySplitView.setSidebar(sv, child)\n";
      s += "            else adw.OverlaySplitView.setContent(sv, child);\n";
      s += "        } else adw.OverlaySplitView.setContent(sv, child);\n";
      return s;
    },
    remove: () => {
      let s = "        const sv: *adw.OverlaySplitView = @ptrCast(@alignCast(parent));\n";
      s += "        if (adw.OverlaySplitView.getSidebar(sv) == child) adw.OverlaySplitView.setSidebar(sv, null)\n";
      s += "        else if (adw.OverlaySplitView.getContent(sv) == child) adw.OverlaySplitView.setContent(sv, null);\n";
      return s;
    },
  },
```
NOTE: verify the exact adw binding method names (`setSidebarWidthFraction`, `setCollapsed`, `setSidebar`, `setContent`, `getSidebar`, `getContent`) against the regenerated `vendor/gobject-bindings` adwaita module — zig-gobject camelCases the C names, and boolean args may be `c_int` (`if (c) 1 else 0`) or Zig `bool` depending on the binding. Grep the binding: `rg -n "pub fn set(Sidebar|Content|Collapsed)" vendor/gobject-bindings` for the real signatures before finalizing the emitter strings.

- [ ] **Step 4: Add the SWIFT_STRUCTURAL + Swift create arm**

In `tools/codegen.ts`:
1. `genSwiftCreateBody` — SplitView maps to `NSSplitViewController`'s view, or more simply an `NSSplitView` with a sidebar-styled item. Best-effort (AppKit fidelity is honestly lower here): create an `NSSplitView` (`isVertical = true`), and wrap the sidebar child in an `NSVisualEffectView` (material `.sidebar`) at append time. Emit:
```swift
    } else if kind == "SplitView" {
        let split = NSSplitView()
        split.isVertical = true
        split.dividerStyle = .thin
        return split
```
2. `SWIFT_STRUCTURAL` — add a SplitView entry branching on `attachedSlot` (decoded in the Swift `attachedPrelude`): sidebar child gets wrapped in an `NSVisualEffectView` (material `.sidebar`, blendingMode `.behindWindow`) then `addArrangedSubview`; content child added directly. Mirror the Grid/TabView entry shape. Since the Swift `attachedPrelude` (codegen.ts ~line 1257) decodes `tabLabel`/`gridRow`/etc., add `slot` decode there too.
NOTE: Mac SplitView is explicitly best-effort/lower-fidelity — the acceptance bar for Mac is "builds + runs + acceptable degradation" (see Task 10), not pixel-native. If `NSSplitView` arranged-subview bookkeeping proves heavy, a simpler `NSStackView` (horizontal) with the sidebar in an `NSVisualEffectView` is an acceptable v1; document whichever is chosen.

- [ ] **Step 5: Regenerate + build both sides**

Run: `nix develop -c bash -c 'bun tools/codegen.ts && rg -n "SplitView|OverlaySplitView" src/generated/widgets.zig && zig build test && zig build'`
Expected: SplitView arms present; build clean. Fix the FIRST error (likely an adw binding method-name mismatch — check the binding grep from Step 3).

- [ ] **Step 6: Conformance coverage**

The conformance suite (`src/conformance.zig`) is schema-driven and runs against the null backend under `zig build test`. Confirm SplitView is exercised (create-defaults, createAndUpdate for `collapsed`, container-ordering). The null backend's structural path may need a `slot`-aware branch or it may be generic — read `src/null_backend.zig`'s append/insertBefore/remove. If the null backend stores children in a flat ordered list (it does — `array.insert(gpa, i, child)`), SplitView's two-slot model is fine at the null level (it just tracks membership; slot semantics are GTK/AppKit-only). Add an explicit conformance assertion if the generic suite doesn't cover attached-prop-bearing containers:
Run: `nix develop -c zig build test 2>&1 | rg -i "splitview|conformance"`
Expected: SplitView appears in the schema-driven conformance run, passing.

- [ ] **Step 7: Commit**

```bash
git add schema/widgets.json src/protocol.zig tools/codegen.ts src/generated/widgets.zig swift/Sources/NDGen/Widgets.swift src/conformance.zig src/null_backend.zig packages/react/src/generated docs/widgets.md
git commit -m "feat(mac): SplitView widget (AdwOverlaySplitView / NSSplitView sidebar+content)"
git show --stat HEAD | grep -q 'src/protocol.zig' || { echo "FAIL"; exit 1; }
```

---

### Task 8: `HeaderBar` widget (title + start/end slots) (Wave 3, after Task 7 committed)

**Files:**
- Modify: `schema/widgets.json` (add HeaderBar; extend the `slot` attached-prop enum with `start`/`end`)
- Modify: `src/protocol.zig` (no new field — `slot` already added in Task 7; verify the enum values are handled)
- Modify: `tools/codegen.ts` (create arm + `STRUCTURAL`/`SWIFT_STRUCTURAL` entries)
- Modify: `src/conformance.zig`

**Interfaces:**
- Consumes: `adw` module, `slot` attached prop (Task 7).
- Produces: `<headerbar>` with a `title?` prop and `slot="start"|"end"` children.

**Widget choice:** `AdwHeaderBar` via `adw_header_bar_new()`; `adw_header_bar_pack_start(self, GtkWidget*)` / `pack_end(self, GtkWidget*)` for slot children; `adw_header_bar_set_title_widget(self, GtkWidget*)` with an `AdwWindowTitle` (`adw_window_title_new(title, subtitle)`) for the title. **Fallback:** `GtkHeaderBar` (same pack_start/pack_end API) if adw proves infeasible — but Task 0 confirmed adw exists.

- [ ] **Step 1: Extend the slot enum + add HeaderBar to schema**

Edit `schema/widgets.json`. First, extend the SplitView `slot` attached-prop enum is per-widget, but the `slot` field in `protocol.Attached` is shared — so HeaderBar's slot values (`start`/`end`) coexist with SplitView's (`sidebar`/`content`) at the struct level. Add the HeaderBar widget with its own attachedProps enum:
```json
    {
      "name": "HeaderBar",
      "intrinsic": "headerbar",
      "container": {
        "childModel": "multi",
        "attachedProps": [
          { "name": "slot", "type": "enum", "values": ["start", "end"], "default": "start" }
        ]
      },
      "props": [
        { "name": "title", "type": "string", "default": "", "appliesTo": "create" },
        { "name": "testID", "type": "string", "appliesTo": "meta" }
      ],
      "events": [],
      "automation": { "role": "toolbar", "textFrom": "title" }
    },
```
NOTE: `collectAttachedProps` (codegen.ts:64-74) unions attached props by NAME across all widgets. Both SplitView and HeaderBar declare an attached prop named `slot` — they must be a compatible type (both `enum`, both a string on the wire). The TS union will type `slot?: "sidebar" | "content" | "start" | "end"` (or `string`, depending on how the union merges enum values — read `collectAttachedProps` and confirm it either widens to `string` or unions the value sets; if it collides on differing `values`, widen the TS type to `string` for `slot` in the emitter). Verify no codegen throw on the duplicate name.

- [ ] **Step 2: Confirm protocol.Attached.slot already handles both enums**

No edit needed — `slot` is a `?[]const u8` (opaque string), so `start`/`end`/`sidebar`/`content` all decode identically. Verify by reading `src/protocol.zig`'s `Attached.fromProps` (from Task 7).

- [ ] **Step 3: Add the create arm + STRUCTURAL entry (Zig)**

In `tools/codegen.ts`:
1. `genZigCreateBody` — HeaderBar arm (title via AdwWindowTitle):
```zig
    } else if (std.mem.eql(u8, kind, "HeaderBar")) {
        const hb = adw.HeaderBar.new();
        if (propStr(props, "title")) |t| {
            const wt = adw.WindowTitle.new(dupeZ(t), "");
            adw.HeaderBar.setTitleWidget(hb, wt.as(gtk.Widget));
        }
        return hb.as(gtk.Widget);
```
2. `STRUCTURAL` — HeaderBar entry branching on `attached.slot`:
```ts
  HeaderBar: {
    append: () => {
      let s = "        const hb: *adw.HeaderBar = @ptrCast(@alignCast(parent));\n";
      s += "        if (attached.slot) |sl| {\n";
      s += "            if (std.mem.eql(u8, sl, \"end\")) adw.HeaderBar.packEnd(hb, child)\n";
      s += "            else adw.HeaderBar.packStart(hb, child);\n";
      s += "        } else adw.HeaderBar.packStart(hb, child);\n";
      return s;
    },
    insertBefore: () => {
      let s = "        _ = before;\n";
      s += "        const hb: *adw.HeaderBar = @ptrCast(@alignCast(parent));\n";
      s += "        if (attached.slot) |sl| {\n";
      s += "            if (std.mem.eql(u8, sl, \"end\")) adw.HeaderBar.packEnd(hb, child)\n";
      s += "            else adw.HeaderBar.packStart(hb, child);\n";
      s += "        } else adw.HeaderBar.packStart(hb, child);\n";
      return s;
    },
    remove: () => "        adw.HeaderBar.remove(@ptrCast(@alignCast(parent)), child);\n",
  },
```
Verify `adw.HeaderBar.packStart`/`packEnd`/`setTitleWidget`/`remove` and `adw.WindowTitle.new` exact names against the binding (`rg -n "pub fn (pack|setTitle|remove)" vendor/gobject-bindings` in the adwaita module).

- [ ] **Step 4: Add SWIFT_STRUCTURAL + Swift create arm**

`genSwiftCreateBody` — HeaderBar on AppKit is best-effort. A real `NSToolbar` is window-integrated and heavy; for M11 use a styled horizontal bar: an `NSVisualEffectView` (material `.titlebar` / `.headerView`) containing an `NSStackView`, with the title as a centered `NSTextField`. Emit that, and `SWIFT_STRUCTURAL.HeaderBar` packs slot children into the stack (start → leading, end → trailing). Document that full NSToolbar integration is v2. Mirror the TabView Swift entry shape.

- [ ] **Step 5: Regenerate + build**

Run: `nix develop -c bash -c 'bun tools/codegen.ts && rg -n "HeaderBar" src/generated/widgets.zig && zig build test && zig build'`
Expected: HeaderBar arms present; build clean.

- [ ] **Step 6: Conformance**

Run: `nix develop -c zig build test 2>&1 | rg -i "headerbar|conformance"`
Expected: HeaderBar in the schema-driven conformance run, passing.

- [ ] **Step 7: Commit**

```bash
git add schema/widgets.json tools/codegen.ts src/generated/widgets.zig swift/Sources/NDGen/Widgets.swift src/conformance.zig packages/react/src/generated docs/widgets.md
git commit -m "feat(mac): HeaderBar widget (AdwHeaderBar / styled NSVisualEffectView bar)"
git show --stat HEAD | grep -q schema/widgets.json || { echo "FAIL"; exit 1; }
```

---

### Task 9: Restyle notes with native chrome + drive + headless leg (Wave 4)

**Files:**
- Modify: `examples/notes/main.tsx` (replace hand-rolled sidebar/headerbar boxes with `<splitview>`/`<headerbar>`; add cssClasses; drop the TextArea ScrollView wrapper)
- Modify: `scripts/notes-drive.ts` (updated testIDs/assertions; add `ND_NAVCHROME_OK`)
- Modify: `scripts/headless-notes.sh` (assert both `ND_NOTES_OK` and `ND_NAVCHROME_OK`)
- Modify: `.github/workflows/ci.yml` (ensure the notes leg runs — it may already)

**Interfaces:**
- Consumes: `<splitview>`, `<headerbar>` (Tasks 7, 8), `cssClasses` (Tasks 2, 6), TextArea floor (Task 4).
- Produces: a notes app that reads as native GNOME + `ND_NAVCHROME_OK` acceptance marker.

- [ ] **Step 1: Restyle main.tsx**

Rewrite `examples/notes/main.tsx`'s layout (read the current 237-line file first). Replace:
- The hand-rolled header `<box>` → `<headerbar title="ND Notes">` with the New-note button in `slot="start"` (or the whole headerbar can sit above the split view). Keep `testID="app-title"` reachable (the AdwWindowTitle carries the title; add a `testID` on the headerbar).
- The hand-rolled two-pane `<box orientation="horizontal">` → `<splitview sidebarWidth={0.28}>` with the sidebar box as `slot="sidebar"` and the detail box as `slot="content"`.
- Sidebar note-list rows: add `cssClasses={["navigation-sidebar"]}` on the list container; the New button `cssClasses={["suggested-action", "pill"]}`; the delete button `cssClasses={["destructive-action"]}`; section headings `cssClasses={["title-4"]}` or `title-2`; the note-count footer `cssClasses={["dimmed"]}` (NOT the deprecated `dim-label`).
- Drop hardcoded `background:` colors from the styled boxes so dark mode works (keep only genuinely needed style props). REMOVE the note-list keyed-remount workaround and verify pin-reorder still lands correctly: the insertBefore reorder bug was fixed post-plan-draft (gtk reorderChildAfter via the codegen emitter, commits 1ff729d/465106f; null backend a20b925) — the drive script's pin leg is the regression proof. Keep the detail-pane `key={selected.id}` remount (that one is idiomatic, not a workaround).
- Remove the `<scrollview minContentHeight={320}>` wrapper around the editor TextArea — TextArea now has its own floor (Task 4). Use `<textarea minContentHeight={320}>` directly. Keep `testID="editor-textarea"`.
- Preserve every existing testID the driver uses: `new-note-button`, `search-input`, `note-row-{id}`, `note-count-label`, `title-input`, `pin-checkbox`, `delete-note-button`, `editor-textarea`, `status-label`, `empty-state-label`.

- [ ] **Step 2: Update the drive script assertions**

Edit `scripts/notes-drive.ts`. Keep the full create/edit/search/pin/delete round-trip and `ND_NOTES_OK`. ADD a native-chrome assertion block: after the baseline `getTree`, assert the tree contains a node whose role is `group` for the splitview and `toolbar` for the headerbar (query by testID on the `<splitview testID="split">` / `<headerbar testID="header">` you add in Step 1). Print `ND_NAVCHROME_OK splitview+headerbar present` on success. Do NOT assert row ORDER after pin (the ordering fix from Task 3 makes getTree order truthful now, but the app-level keyed-remount already sidesteps insertBefore — keep the membership-only assertion the driver already has). Keep the `setValueRetrying` helper.

- [ ] **Step 3: Update the headless script to assert both markers**

Edit `scripts/headless-notes.sh` line 35 (the `ND_NOTES_OK` grep) to also require the new marker:
```bash
grep -q "ND_NOTES_OK" "$XDG_RUNTIME_DIR/drive.log" || { echo "FAIL: driver did not report success"; exit 1; }
grep -q "ND_NAVCHROME_OK" "$XDG_RUNTIME_DIR/drive.log" || { echo "FAIL: native chrome not present"; exit 1; }
```

- [ ] **Step 4: Run the notes leg**

Run: `nix develop -c ./scripts/headless-notes.sh`
Expected: prints the drive log with `ND_NOTES_OK` AND `ND_NAVCHROME_OK`, both screenshots are valid PNGs, and `headless notes: OK`. If SplitView/HeaderBar fail to mount, the host log will show a create error — check `$XDG_RUNTIME_DIR/drive.log` and the host `$LOG`. Fix the FIRST error.

- [ ] **Step 5: Capture the acceptance screenshot pair**

The drive script already writes `notes-baseline.png` + `notes-final.png` to `$XDG_RUNTIME_DIR`. Confirm they render the native chrome (open them or check dimensions). These are the acceptance artifact per the milestone. They live in the gitignored runtime dir (not committed, per m5c/m6 precedent) — the marker + green script IS the committed evidence.

- [ ] **Step 6: Confirm CI runs the notes leg**

Run: `nix develop -c rg -n "headless-notes" .github/workflows/ci.yml`
Expected: the notes leg is invoked. If absent, add `./scripts/headless-notes.sh` after the `headless-m10` step in `ci.yml` (mirror how the other headless legs are listed).

- [ ] **Step 7: Commit**

```bash
git add examples/notes/main.tsx scripts/notes-drive.ts scripts/headless-notes.sh .github/workflows/ci.yml
git commit -m "feat(notes): restyle with SplitView + HeaderBar + Adwaita cssClasses (native GNOME look)"
git show --stat HEAD | grep -q examples/notes/main.tsx || { echo "FAIL"; exit 1; }
```

---

### Task 10: Mac leg + integration (Wave 4, final)

**Files:**
- Create: `scripts/mac/mac-m11.sh` (mirror `scripts/mac/mac-m6.sh`)
- Modify: `.github/workflows/mac.yml` (add the m11 leg, non-blocking per precedent)
- Modify: `CLAUDE-activeContext.md` (add the M11 entry)
- Run: the full gate + mac legs

**Interfaces:**
- Consumes: everything.
- Produces: `MAC_M11_OK` marker (Mac builds + runs the restyled notes app with acceptable degradation), updated activeContext, green full gate.

**Mac scope (honest):** the acceptance bar for Mac is "the notes app BUILDS and RUNS with acceptable degradation" — SplitView maps to `NSSplitView`/`NSVisualEffectView` best-effort, HeaderBar to a styled bar (not full NSToolbar), cssClasses is a no-op. The Mac must not crash on the new widgets; pixel-native Mac chrome is explicitly deferred to a follow-up.

- [ ] **Step 1: Write the Mac m11 driver script**

Create `scripts/mac/mac-m11.sh` mirroring `scripts/mac/mac-m6.sh`'s structure (ssh heredoc; `ar x`/`libtool` repack of `libnd.a`; swiftc link; launch the notes app in-session; drive it). It must: sync the tree to the Mac (`mac-sync.sh`), build (`mac-build.sh` — which repacks the archive), launch `examples/notes/main.tsx` under the Swift shell with `NATIVE_AUTOMATION=1`, run a minimal drive (window appears, splitview + headerbar nodes present in getTree, one note created), and print `MAC_M11_OK` on success. Reuse the `notes-drive.ts` automation client but gate the chrome-role assertions so Mac's degraded roles don't fail the run (Mac SplitView/HeaderBar may report different roles — assert node PRESENCE by testID, not exact native role).

- [ ] **Step 2: Run the Mac leg over ssh**

Run: `ssh macbook 'bash -euo pipefail -s' < scripts/mac/mac-m11.sh 2>&1 | grep -v 'bind\[' | tail -30`
(Filter the harmless port-forward bind noise.)
Expected: `MAC_M11_OK`. **If the Mac cannot be reached (ssh auth / no access from this box):** STOP and hand this step to the owner via `! ssh macbook ...` — do NOT work around it. The Mac leg is non-blocking in CI (per the mac.yml precedent); a deferred first live run is acceptable and must be recorded honestly in activeContext.

- [ ] **Step 3: Add the m11 leg to mac.yml (non-blocking)**

Edit `.github/workflows/mac.yml`, adding the m11 script invocation as a `continue-on-error` step (mirror how mac-m6/mac-m9 are wired).

- [ ] **Step 4: Run the FULL Linux gate**

Run the Global-Constraints full gate command (top of this plan). Expected: every leg green, codegen-freshness diff empty. Fix the FIRST failure; do not proceed until fully green.

- [ ] **Step 5: Update activeContext**

Edit `CLAUDE-activeContext.md`, adding an M11 entry to the State section summarizing: cssClasses escape hatch (allowlist-validated against Adwaita classes; GTK real via addCssClass, AppKit no-op); Adwaita runtime (AdwApplication + AdwApplicationWindow, dark-mode-aware, hardcoded style colors are explicit non-adapting overrides); SplitView (AdwOverlaySplitView, chosen over NavigationSplitView to avoid AdwNavigationPage wrapping) + HeaderBar (AdwHeaderBar) with a shared `slot` attached prop (hand-maintained in `protocol.Attached`, NOT codegen-derived); TextArea minContentHeight floor; automation getTree ordering fix (Tree now keeps an ordered per-parent children list, replacing the hashmap-iteration bug). Record hard-won facts: the Adw binding regen path (`-Dmodules=Gtk-4.0,Adw-1`), the module name zig-gobject assigned, and whether the Mac leg got a live run or was deferred. Note the one suitability finding NOT addressed in M11 as an explicit descope: schema-level create-only props (e.g. Button.label, appliesTo=create) still cannot be updated post-mount by design; the related update-op empty-kind bug and the insertBefore reorder bug were BOTH already fixed pre-M11 (2ba8701; 1ff729d/465106f/a20b925).

- [ ] **Step 6: Final commit**

```bash
git add scripts/mac/mac-m11.sh .github/workflows/mac.yml CLAUDE-activeContext.md
git commit -m "feat(mac): m11 mac verification leg + integration; activeContext M11 entry"
git show --stat HEAD | grep -q mac-m11.sh || { echo "FAIL"; exit 1; }
```

NOTE: per the owner's memory-bank rule, `CLAUDE-activeContext.md` is normally EXCLUDED from commits. The owner's convention for this repo has been to commit activeContext updates as part of milestone integration (M6b/M9/M10 all did). Confirm with the reviewer before the final commit whether activeContext should be in the commit or updated out-of-band; if excluded, drop it from the `git add` above and update it separately.

---

## Self-Review

**Spec coverage:**
- Scope 1 (cssClasses escape hatch, allowlist + Levenshtein): Task 2 (TS + codegen), Task 6 (GTK addCssClass, AppKit no-op). ✓
- Scope 2 (Adwaita runtime, investigate bindings first): Task 0 (research + regen, hard fallback at Step 1), Task 5 (AdwApplication + dark-mode doc). ✓
- Scope 3 (SplitView + HeaderBar, both backends + null + conformance + slot attached prop): Tasks 7, 8. ✓ (AdwOverlaySplitView chosen with rationale; GtkPaned fallback stated; AppKit best-effort scoped.)
- Scope 4 (restyle notes, delete hand-rolled approximations, headless green + screenshots, Mac builds): Task 9 (Linux), Task 10 (Mac). ✓
- Scope 5 repairs (TextArea zero-height; automation getTree ordering): Task 4, Task 3. ✓
- Waves on disjoint files, single-owner codegen/schema/protocol/build serialized: enforced via the wave NOTEs. ✓
- Zig 0.16 + env hard facts (ssh heredoc, ar/libtool repack, weston socket uniqueness, pathspec commits + git show --stat): in Global Constraints + per-task commit steps. ✓
- Research-first Task 0 with hard fallback decision point: Step 1. ✓

**Placeholder scan:** No TBD/TODO/"handle edge cases" — every code step shows the code; binding-method-name verification steps are explicit (grep the regenerated bindings) rather than assumed.

**Type consistency:** `slot: ?[]const u8` is added once in Task 7 and reused (not redefined) in Task 8. `childrenOf`/`recordAppend`/`recordInsertBefore`/`recordRemove`/`deinitChildren` are defined in Task 3 Step 3 and used consistently. `validateCssClasses`/`cssClassSpec`/`CssClassError` names match across Tasks 2 and 6. `ND_NAVCHROME_OK` is produced in Task 9 Step 2 and consumed in Step 3.

**Open risks flagged in-plan:** (a) exact adw binding method names/bool-arg conventions — mitigated by explicit grep-the-binding verification steps; (b) `collectAttachedProps` merging two `slot` enums with different value sets — flagged in Task 8 Step 1 with a widen-to-string fallback; (c) whether cssClasses can reach the Mac backend without a vtable change — flagged in Task 6 Step 5 with a drop-on-Mac fallback; (d) Mac ssh reachability — flagged in Task 10 Step 2 with a hand-to-owner fallback; (e) activeContext commit-inclusion convention — flagged in Task 10 Step 6.
