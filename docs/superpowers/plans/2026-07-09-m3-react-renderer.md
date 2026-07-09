# M3 — React Renderer (`@nativedesktop/react`): Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Parallelism note.** Two independent tracks:
> - **Zig track (Tasks 1–2):** extend NDP ops (`remove`, `insertBefore`, `hide`, `unhide`) on the host + `runtime/ndp.ts` type. Independently testable via golden-frame tests + the M2 headless harness; touches only `src/*.zig`, `runtime/ndp.ts`, `build.zig`.
> - **Renderer track (Tasks 3–7):** the `@nativedesktop/react` package + counter demo. Touches only `packages/react/`, `examples/counter/`, root `package.json`/`tsconfig.base.json`.
> These two tracks share **no files** and can run concurrently on different agents against the Interfaces blocks. **Task 8 (headless-m3 + CI) integrates both and must run last.** Within the renderer track, Task 4 (host config) depends on Task 3 (package scaffold + reconciler install); Task 5 (JSX types) is parallel to Task 4; Tasks 6–7 depend on 4+5.
> **Task 9 (binary-encoding spec document)** is a standalone doc deliverable with no code dependency — parallelizable with everything.

**Goal:** From the M2 tree-over-NDP host, ship `@nativedesktop/react`: a bundled `react@19.2.x` + `react-reconciler@0.33.0` custom renderer in **mutation mode** whose render-phase host config is pure (JS descriptors with generation-tagged monotonic node IDs) and whose commit-phase methods accumulate NDP ops into **one `CommitBatch` per React commit**, flushed as a single `sendCommit` in `resetAfterCommit`. An interactive TSX counter app (`examples/counter/main.tsx`) drives live GTK widgets: `useState`+`onClick` increments a label, a `Suspense` block resolves after ~1s (proving `hide`/`unhide`), and a `startTransition`-driven label proves update-priority wiring. The host gains four new ops (`remove`, `insertBefore`, `hide`, `unhide`, spec §4 verbatim). All green under headless CI. Separately, the **binary command-buffer encoding SPEC DOCUMENT** (spec D4) is written — schema only, no implementation.

**Architecture:** Unchanged two-process topology (spec D1). The Zig host is authoritative over the retained tree and already applies `create`/`append`/`setText`/`update` (M2). M3 adds the four mutation ops it was missing so a full React reconciler (which reorders and removes children, and hides/shows subtrees for Suspense) can drive it. The Bun child stops being a hand-written demo script and becomes a React app: `examples/counter/main.tsx` imports `@nativedesktop/react`, which wraps the M2 `Ndp` client. The renderer's host config never touches the socket during render (concurrent React discards work-in-progress trees — real ops would leak host widgets); it emits ops only from commit-phase callbacks into a batch that `resetAfterCommit` flushes once. Node IDs are assigned monotonically and tagged with a generation counter (`(generation << 24) | seq`, or a documented packing) so a future re-mount can garbage-collect orphaned-generation widgets host-side.

**Tech Stack:** Zig 0.16.0 (exact), the vendored zig-gobject bindings at `vendor/gobject-bindings` (glib2/gobject2/gio2/gtk4), Bun 1.3.13 (from the flake), `react@19.2.x` + `react-reconciler@0.33.0` (exact-pinned, bundled in `@nativedesktop/react`), GTK4 ≥ 4.20 (devshell 4.22.4), weston headless + `GSK_RENDERER=cairo` for CI, GitHub Actions. TypeScript strict throughout; type-checked with `bunx tsc --noEmit`.

## Global Constraints

Carried over from M1/M2, unchanged:

- Zig is exactly `0.16.0`; `build.zig`'s `checkZigVersion()` guard stays and fails loudly otherwise.
- Bun is pinned `1.3.13` (from the flake devshell).
- No `@cImport` anywhere; all GTK access goes through the vendored zig-gobject modules imported as `glib`/`gobject`/`gio`/`gtk` (names fixed in `build.zig`).
- No hand-written per-widget C bindings (spec D6); only the vendored generated modules.
- Headless CI uses `weston --backend=headless` + `GSK_RENDERER=cairo` — NOT Broadway, NOT Xvfb/X11.
- Commit style: short imperative lowercase subject (e.g. `feat: add hide/unhide/remove/insertBefore ops`). No co-author trailers, no body unless closing an issue.
- All commands run inside the devshell (direnv activates it in the repo; in CI, `nix develop -c`).
- Host prints machine-greppable markers to **stderr**: existing `ND_CHILD_CONNECTED`, `ND_HELLO_OK`, `ND_COMMIT_APPLIED commitId=<n>`, `ND_CHILD_EXITED`; M3 adds `ND_HIDE id=<n>` / `ND_UNHIDE id=<n>` / `ND_REMOVE id=<n>` inside the apply path (used by the headless-m3 Suspense assertion). Scripts capture `2>&1`.
- TypeScript is strict. `runtime/ndp.ts` and `runtime/m2-demo.ts` stay where they are. New package/app code lives under `packages/react/` and `examples/counter/` and must pass `bunx tsc --noEmit` with strict.

### M3-new constraints (owner decisions, verbatim)

- **Package layout.** A **root `package.json`** with `"workspaces": ["packages/*", "examples/*"]` and `"private": true` is added (the repo has none today). `packages/react/` is the `@nativedesktop/react` package; `examples/counter/` is the demo app depending on `@nativedesktop/react` via the workspace (`"@nativedesktop/react": "workspace:*"`). `runtime/ndp.ts` stays put; `packages/react` imports it by **relative path** (`../../runtime/ndp.ts`) — it is framework-internal transport, not a published entrypoint, so it is not re-exported.
- **Exact pins, no carets.** `packages/react/package.json` lists `react`, `react-dom` (peer for JSX runtime types only — see Task 5 note), `react-reconciler`, and their `@types/*` as **exact** versions (no `^`). Verified pairing (npm registry, research entry `react-renderer`): **react-reconciler `0.33.0` has `peerDependency react ^19.2.0`**; the lockstep table is `0.31↔19.0, 0.32↔19.1, 0.33↔19.2`. So the pinned pair is **`react@19.2.x` + `react-reconciler@0.33.0`**. `bun.lock` is **committed**; `node_modules/` stays gitignored (already in `.gitignore`).
- **Purity.** Render-phase host config methods (`createInstance`, `createTextInstance`, `appendInitialChild`, `finalizeInitialChildren`) are **side-effect-free**: they return cheap JS descriptor objects and assign monotonic generation-tagged node IDs — they NEVER call `sendCommit` or touch the socket. Real ops are emitted ONLY from commit-phase methods (`appendChild`, `appendChildToContainer`, `insertBefore`, `insertInContainerBefore`, `removeChild`, `removeChildFromContainer`, `commitUpdate`, `commitTextUpdate`, `hideInstance`, `unhideInstance`, `hideTextInstance`, `unhideTextInstance`) into a batch accumulator flushed in `resetAfterCommit` as a single `sendCommit`.
- **`commitUpdate` self-diffs.** `prepareUpdate` is GONE in React 19; `commitUpdate(instance, type, oldProps, newProps, ...)` computes its own changed-prop set and emits one `update` op (or a `setText` for the text child, per the text mapping below).
- **Host context.** `getRootHostContext`/`getChildHostContext` return non-null sentinels (React 19 treats `null` as "missing context").
- **Update priority.** `resolveUpdatePriority`/`getCurrentUpdatePriority`/`setCurrentUpdatePriority` are implemented against incoming event priorities: discrete→`DiscreteEventPriority`, continuous→`ContinuousEventPriority`, else `DefaultEventPriority` — imported from **`react-reconciler/constants`**. A module-scoped "current update priority" cell holds the value the reconciler sets; the event dispatcher seeds it from the NDP event's `priority` field before invoking the handler so `startTransition` and discrete-click lanes behave correctly.
- **Text mapping (owner's call, specified exactly).** v1 maps JSX text children to a Label's `text` prop. A `<label>Clicks: {n}</label>` compiles to a `label` host instance whose single string/number child becomes the Label's `text`. Concretely: `createTextInstance` returns a text descriptor; when a `label` instance's children are text, the renderer folds them into the Label `text` prop at `createInstance`/`commitUpdate`/`commitTextUpdate` time and emits `setText` on the label's node ID. There is **no** standalone GTK text node — GTK Labels own their text. `shouldSetTextContent(type, props)` returns `true` for `label` so React treats a label's string children as host text content (no separate text instances created), which is the cleanest mapping and the counter demo's changing count flows through `commitUpdate`→`setText`. (Rationale: mirrors how react-dom treats `<option>`/leaf text; avoids inventing a GTK widget with no backing.)
- **Event routing.** The `Ndp.onEvent` stream dispatches to per-node handler props via a registry keyed by node ID inside the renderer. `commitUpdate` registers/updates/removes the `onClick` handler for a node; `removeChild` unregisters. On an incoming `event {nodeId, name, priority}`, the dispatcher looks up the node's handler, seeds the current update priority from `priority` (discrete for clicks), and calls it. `setState` inside the handler therefore runs on the discrete lane.
- **Host-side new ops (Zig).** `remove {op:"remove", id}`, `insertBefore {op:"insertBefore", parent, child, before}`, `hide {op:"hide", id}`, `unhide {op:"unhide", id}` — spec §4 names verbatim. Extend `src/protocol.zig` `Op` struct, `src/tree.zig` `apply`, `src/gtk_backend.zig` (visibility via `gtk.Widget.setVisible`; Box remove/insert via `gtk.Box.remove`/`gtk.Box.insertChildAfter`), and `runtime/ndp.ts` `Op` type. Golden-frame tests in `protocol.zig` mirror the existing ones.
- **JSX.** Typed lowercase intrinsics for the M2 widget set: `window {title, defaultWidth, defaultHeight}`, `box {orientation, spacing}`, `label {text}`, `button {label, onClick}`. `react-jsx` transform via `tsconfig` `jsx: "react-jsx"`.
- **Demo.** `examples/counter/main.tsx`: `window > vertical box > [label "Clicks: {n}", button increment, Suspense(fallback label "loading...") wrapping a component that `use()`s a ~1s promise, a startTransition-driven label]`. For headless CI a `setInterval`-driven uptime state update keeps `ND_COMMIT_APPLIED` markers flowing, and the resolved-Suspense content produces a distinctive commit. Runs under the host via `ND_SCRIPT=examples/counter/main.tsx` (runtime.zig reads `ND_SCRIPT`, default `runtime/m2-demo.ts`, and spawns `bun <script>`; bun runs TSX natively).
- **headless-m3.sh** mirrors headless-m2.sh with a **unique weston socket name** and asserts handshake + ≥3 commits + a Suspense-resolution marker (the exact grep is specified in Task 8). CI appends one step.

### Landed-code reality (authoritative — read before writing)

These differ from the M2 *plan text*; the **landed code** is the contract:

| Fact | Landed reality (file:line) |
|---|---|
| `Runtime.start` signature | `start(gpa, app, tree, parent_env: *const std.process.Environ.Map, real_environ: std.process.Environ)` — `src/runtime.zig:28`. **M3 changes nothing here.** |
| Env / `ND_SCRIPT` | `parent_env.get("ND_SCRIPT") orelse "runtime/m2-demo.ts"`, spawned `bun <script>` — `src/runtime.zig:63`. M3 sets `ND_SCRIPT=examples/counter/main.tsx` at run time; no code change. |
| Writer mutex | `std.Io.Mutex` (`.lockUncancelable(io)` / `.unlock(io)`), not `std.Thread.Mutex` — `src/runtime.zig:21,94`. |
| Commit apply path | reader thread `marshalCommit` → `glib.MainContext.default().invokeFull(...)` → `applyOnUi` → `tree.apply(batch)` — one closure per batch — `src/runtime.zig:177`. **New ops require no runtime.zig change**; they are just new `Op` variants `tree.apply` dispatches. |
| `tree.apply` op dispatch | if-chain on `op.op` string (`create`/`append`/`setText`/`update`) + `ND_WARN unknown op` fallthrough, then `ND_COMMIT_APPLIED commitId={d}` — `src/tree.zig:21`. M3 adds four `else if` arms. |
| Backend | `createWidget`/`appendChild`/`setText`/`applyProps`/`connectButtonClick`; single `the_window: ?*gtk.Window`; arena for null-terminated dupes — `src/gtk_backend.zig`. |
| `Op` struct | permissive optional-field struct, `op: []const u8` discriminator — `src/protocol.zig:36`. Needs a `before: ?u32` field added for `insertBefore`. |
| `runtime/ndp.ts` `Op` | union of `create`/`append`/`setText`/`update` — `runtime/ndp.ts:6`. Needs `remove`/`insertBefore`/`hide`/`unhide` arms. `Ndp` exposes `static connect()`, `handshake(runtime)`, `sendCommit(Omit<CommitBatch,"type">)`, `onEvent(cb)`, `ping()`. |
| `build.zig` | single `gtk_imports` array reused by exe + `tests` (root `src/main.zig`) + `protocol_tests` (root `src/protocol.zig`, no GTK). M3 adds `tree`/`gtk_backend` are already compiled transitively via `main.zig`; the new golden tests live in `protocol.zig` so **no build.zig change is needed** unless a standalone tree test is added (it is not). |

### Verified-symbol conventions (re-verify inside the devshell before pasting)

GTK4 symbols for the new ops, confirmed present in `vendor/gobject-bindings/src/gtk4/gtk4.zig` this session:

| Need | Symbol | Verify command |
|---|---|---|
| Hide / show a widget | `gtk.Widget.setVisible(widget, c_int)` (`gtk_widget_set_visible`) | `rg -n "gtk_widget_set_visible\b" vendor/gobject-bindings/src/gtk4/gtk4.zig` → `58367` |
| Read visibility | `gtk.Widget.getVisible(widget) c_int` | `rg -n "gtk_widget_get_visible\b" vendor/gobject-bindings/src/gtk4/gtk4.zig` → `57597` |
| Remove a Box child | `gtk.Box.remove(box, child)` (`gtk_box_remove`) | `rg -n "gtk_box_remove\b" vendor/gobject-bindings/src/gtk4/gtk4.zig` → `3310` |
| Insert a Box child after a sibling | `gtk.Box.insertChildAfter(box, child, ?sibling)` (`gtk_box_insert_child_after`) | `rg -n "gtk_box_insert_child_after\b" vendor/gobject-bindings/src/gtk4/gtk4.zig` → `3298` |
| Prepend (insert at head; `insertBefore` with the first child) | `gtk.Box.prepend(box, child)` (`gtk_box_prepend`) | `rg -n "gtk_box_prepend\b" vendor/gobject-bindings/src/gtk4/gtk4.zig` → `3302` |

**`insertBefore` mapping.** GTK's Box API inserts *after* a sibling (`insertChildAfter(child, sibling)`) or at the head (`prepend`). React's `insertBefore(parent, child, beforeChild)` means "place `child` immediately before `beforeChild`". The host translates: find the widget preceding `before` in the box and `insertChildAfter(child, that_prev)`; if `before` is the first child, `prepend(child)`. Because the host does not keep sibling order metadata beyond GTK's own child list, use `gtk.Widget.getPrevSibling(before_widget)` (`gtk_widget_get_prev_sibling`) to find the anchor. Verify: `rg -n "gtk_widget_get_prev_sibling\b" vendor/gobject-bindings/src/gtk4/gtk4.zig`. If `before`'s prev sibling is null, `prepend`; else `insertChildAfter(child, prev)`. This is O(1) using the live GTK child list — no host-side ordering bookkeeping.

react-reconciler API surface (confirmed at implement-time by reading the installed package — see Task 3 Step 3, which dumps the required host-config keys rather than trusting this table):

| Need | Symbol | Source |
|---|---|---|
| Reconciler factory | `import Reconciler from "react-reconciler"` (default export, a function taking the host config) | `packages/react/node_modules/react-reconciler/index.js` |
| Priority constants | `import { DiscreteEventPriority, ContinuousEventPriority, DefaultEventPriority } from "react-reconciler/constants"` | `.../react-reconciler/constants.js` |
| Container create | `reconciler.createContainer(...)` → `reconciler.updateContainer(element, container, null, cb)` | reconciler README |

The reconciler README explicitly disclaims host-config stability. **Do not trust any blog post or this plan's method list as complete** — Task 4 Step 1 runs the reconciler once behind a `Proxy` host config that logs every accessed key, producing the authoritative required-method list for the installed 0.33.0.

---

### Task 1: Extend NDP ops on the host (Zig) — `remove`, `insertBefore`, `hide`, `unhide` (TDD golden frames first)

**Files:**
- Modify: `src/protocol.zig` (add `before` field to `Op`; add golden-frame tests)
- Modify: `src/tree.zig` (four new `else if` arms in `apply`)
- Modify: `src/gtk_backend.zig` (visibility + box remove/insert helpers)

**Interfaces:**
- Consumes: `protocol.Op`, the vendored `gtk` module.
- Produces:
  - `protocol.Op` gains `before: ?u32 = null` (for `insertBefore`). All other fields unchanged; the discriminator stays `op: []const u8`.
  - `gtk_backend`:
    - `pub fn removeChild(parent: *gtk.Widget, child: *gtk.Widget) void` — Box `remove`; if parent is `the_window`, `gtk.Window.setChild(win, null)`.
    - `pub fn insertBefore(parent: *gtk.Widget, child: *gtk.Widget, before: ?*gtk.Widget) void` — see the `insertBefore` mapping above.
    - `pub fn setVisible(widget: *gtk.Widget, visible: bool) void` — `gtk.Widget.setVisible(widget, @intFromBool(visible))`.
  - `tree.apply` handles `remove`/`insertBefore`/`hide`/`unhide` and prints `ND_REMOVE id=<n>` / `ND_HIDE id=<n>` / `ND_UNHIDE id=<n>` markers (used by headless-m3). `remove` also removes the id from `self.nodes`.

- [ ] **Step 1: Verify the GTK symbols, then add the golden-frame tests to `src/protocol.zig` FIRST**

Run:
```bash
nix develop -c bash -c 'rg -n "gtk_widget_set_visible\b|gtk_box_remove\b|gtk_box_insert_child_after\b|gtk_box_prepend\b|gtk_widget_get_prev_sibling\b" vendor/gobject-bindings/src/gtk4/gtk4.zig'
```
Expected: five hits. If any symbol drifted, adjust the backend body in Step 3; the op wire format (the golden test) is the contract and does not change.

Add the `before` field to `Op` (right after the `child` field at `src/protocol.zig:45`):
```zig
    // insertBefore
    before: ?u32 = null,
```

Append these golden-frame tests to `src/protocol.zig` (mirror the existing `commitBatch ... decodes with field names verbatim` test):
```zig
test "commitBatch with remove/insertBefore/hide/unhide decodes verbatim" {
    const gpa = std.testing.allocator;
    const doc =
        \\{"type":"commitBatch","commitId":7,"generation":1,"ops":[
        \\  {"op":"insertBefore","parent":2,"child":9,"before":3},
        \\  {"op":"remove","id":4},
        \\  {"op":"hide","id":5},
        \\  {"op":"unhide","id":5}
        \\]}
    ;
    const parsed = try std.json.parseFromSlice(CommitBatch, gpa, doc, .{ .ignore_unknown_fields = true });
    defer parsed.deinit();
    try std.testing.expectEqual(@as(u32, 1), parsed.value.generation);
    try std.testing.expectEqual(@as(usize, 4), parsed.value.ops.len);
    try std.testing.expectEqualStrings("insertBefore", parsed.value.ops[0].op);
    try std.testing.expectEqual(@as(u32, 2), parsed.value.ops[0].parent.?);
    try std.testing.expectEqual(@as(u32, 9), parsed.value.ops[0].child.?);
    try std.testing.expectEqual(@as(u32, 3), parsed.value.ops[0].before.?);
    try std.testing.expectEqualStrings("remove", parsed.value.ops[1].op);
    try std.testing.expectEqual(@as(u32, 4), parsed.value.ops[1].id.?);
    try std.testing.expectEqualStrings("hide", parsed.value.ops[2].op);
    try std.testing.expectEqualStrings("unhide", parsed.value.ops[3].op);
    try std.testing.expectEqual(@as(u32, 5), parsed.value.ops[3].id.?);
}
```

Run: `nix develop -c zig build test` — the new test must **pass** immediately once `before` is added (it is pure decode; the "failing first" discipline here is that omitting the `before` field makes `ops[0].before.?` unwrap-null-panic). Fix the first failure only.

- [ ] **Step 2: Add the four op arms to `src/tree.zig` `apply`**

Insert after the existing `update` arm (before the `else` unknown-op fallthrough at `src/tree.zig:40`):
```zig
            } else if (std.mem.eql(u8, op.op, "insertBefore")) {
                const parent = self.nodes.get(op.parent.?) orelse continue;
                const child = self.nodes.get(op.child.?) orelse continue;
                const before: ?*gtk.Widget = if (op.before) |b| self.nodes.get(b) else null;
                backend.insertBefore(parent, child, before);
            } else if (std.mem.eql(u8, op.op, "remove")) {
                const child = self.nodes.get(op.id.?) orelse continue;
                if (child.getParent()) |parent| backend.removeChild(parent, child);
                _ = self.nodes.remove(op.id.?);
                std.debug.print("ND_REMOVE id={d}\n", .{op.id.?});
            } else if (std.mem.eql(u8, op.op, "hide")) {
                const widget = self.nodes.get(op.id.?) orelse continue;
                backend.setVisible(widget, false);
                std.debug.print("ND_HIDE id={d}\n", .{op.id.?});
            } else if (std.mem.eql(u8, op.op, "unhide")) {
                const widget = self.nodes.get(op.id.?) orelse continue;
                backend.setVisible(widget, true);
                std.debug.print("ND_UNHIDE id={d}\n", .{op.id.?});
```
Verify `gtk.Widget.getParent` exists: `rg -n "gtk_widget_get_parent\b" vendor/gobject-bindings/src/gtk4/gtk4.zig`. If the return is `?*gtk.Widget`, the `if (child.getParent())` capture is correct; adjust if the binding names it `getParent`/`parent`.

- [ ] **Step 3: Add the backend helpers to `src/gtk_backend.zig`**

Append after `applyProps`:
```zig
pub fn removeChild(parent: *gtk.Widget, child: *gtk.Widget) void {
    if (the_window) |win| {
        if (parent == win.as(gtk.Widget)) {
            gtk.Window.setChild(win, null);
            return;
        }
    }
    const box: *gtk.Box = @ptrCast(@alignCast(parent));
    gtk.Box.remove(box, child);
}

pub fn insertBefore(parent: *gtk.Widget, child: *gtk.Widget, before: ?*gtk.Widget) void {
    if (the_window) |win| {
        if (parent == win.as(gtk.Widget)) {
            gtk.Window.setChild(win, child);
            return;
        }
    }
    const box: *gtk.Box = @ptrCast(@alignCast(parent));
    if (before) |b| {
        const prev = gtk.Widget.getPrevSibling(b);
        gtk.Box.insertChildAfter(box, child, prev); // prev == null => head
    } else {
        gtk.Box.append(box, child);
    }
}

pub fn setVisible(widget: *gtk.Widget, visible: bool) void {
    gtk.Widget.setVisible(widget, @intFromBool(visible));
}
```
`gtk.Box.insertChildAfter(box, child, null)` inserts at the head (GTK contract: a null sibling means "the beginning"), so the explicit `prepend` is unnecessary — one call covers both "before the first child" and "before a middle child". Verify the null-sibling semantics if in doubt by reading the doc comment above `gtk_box_insert_child_after` in the bindings.

- [ ] **Step 4: Extend `runtime/ndp.ts` `Op` type to match**

In `runtime/ndp.ts`, extend the `Op` union (`runtime/ndp.ts:6`) to:
```ts
type Op =
  | { op: "create"; id: number; widget: "Window" | "Box" | "Label" | "Button"; props: Record<string, unknown> }
  | { op: "append"; parent: number; child: number }
  | { op: "insertBefore"; parent: number; child: number; before: number | null }
  | { op: "remove"; id: number }
  | { op: "setText"; id: number; text: string }
  | { op: "update"; id: number; props: Record<string, unknown> }
  | { op: "hide"; id: number }
  | { op: "unhide"; id: number };
```
No other `ndp.ts` change; the transport already sends whatever `sendCommit` is handed. Run `nix develop -c bash -c 'cd runtime && bunx tsc --noEmit'` — expected clean (the M2 demo does not use the new ops).

- [ ] **Step 5: Build + test**

Run: `nix develop -c bash -c 'zig build test && zig build'`
Expected: golden tests green (including the new one); host binary builds. Fix the first error only — the likely one is a `getParent`/`getPrevSibling` binding name; re-verify with `rg` and adjust.

- [ ] **Step 6: Commit**
```bash
git add src/protocol.zig src/tree.zig src/gtk_backend.zig runtime/ndp.ts
git commit -m "feat: add hide/unhide/remove/insertBefore ndp ops on the host"
```

### Task 2: Regression-check the new ops against the M2 demo path (no new demo yet)

**Files:** none created; this is a verification task that proves the host still round-trips before the renderer exists, so Task 1 can be signed off independently of the renderer track.

**Interfaces:**
- Consumes: Task 1 host, the existing `runtime/m2-demo.ts` (unchanged), the M2 headless harness.
- Produces: confidence that `zig build` + `headless-m2.sh` + `kill9-test.sh` are still green after the op extension.

- [ ] **Step 1: Run the full M2 gate against the extended host**

Run:
```bash
nix develop -c bash -c 'zig build test && zig build && ./scripts/headless-m2.sh && ./scripts/kill9-test.sh'
```
Expected: all green — `headless m2: OK (N commits)` (N≥3), `kill9: OK`. The extended host applies the M2 tree exactly as before; the new op arms are dormant until the renderer emits them. If anything regressed, fix in Task 1 before proceeding. No commit (nothing changed here).

> **Zig track (Tasks 1–2) is complete and independently green here.** The renderer track (Tasks 3–7) can proceed in parallel from the start.

---

### Task 3: Workspace scaffold + `@nativedesktop/react` package skeleton + exact-pinned reconciler install

**Files:**
- Create: `package.json` (repo root — workspace manifest)
- Create: `tsconfig.base.json` (repo root — shared strict base)
- Create: `packages/react/package.json`
- Create: `packages/react/tsconfig.json`
- Create: `packages/react/src/index.ts` (barrel; stub for now)
- Create/commit: `bun.lock`

**Interfaces:**
- Consumes: nothing (new tree).
- Produces:
  - Root `package.json` with `"workspaces": ["packages/*", "examples/*"]`, `"private": true`.
  - `tsconfig.base.json` centralizing strict + `moduleResolution: "bundler"` + `jsx: "react-jsx"` + `types: ["bun"]`; per-package `tsconfig.json` files `extends` it. (Chosen over a single root tsconfig because `packages/react` needs `jsx: "react-jsx"` + `jsxImportSource: "react"` and `runtime/` uses a different `include`/`types` set — a shared base with per-package overrides is the least-friction, matching the existing `runtime/tsconfig.json` prior art.)
  - `@nativedesktop/react` package with react/react-reconciler as **exact** dependencies and a committed lockfile.

- [ ] **Step 1: Write the root workspace manifest and shared tsconfig base**

`package.json` (root):
```json
{
  "name": "nativedesktop-monorepo",
  "private": true,
  "workspaces": ["packages/*", "examples/*"]
}
```

`tsconfig.base.json` (root):
```json
{
  "compilerOptions": {
    "target": "ESNext",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "lib": ["ESNext", "DOM"],
    "jsx": "react-jsx",
    "jsxImportSource": "react",
    "strict": true,
    "noUncheckedIndexedAccess": true,
    "noImplicitOverride": true,
    "skipLibCheck": true,
    "esModuleInterop": true,
    "allowImportingTsExtensions": true,
    "noEmit": true
  }
}
```
`"lib": ["...", "DOM"]` is required because `@types/react`'s JSX runtime references DOM lib types; `skipLibCheck` keeps that cheap. `allowImportingTsExtensions` lets `packages/react` import `../../runtime/ndp.ts` by its real path under bun.

- [ ] **Step 2: Verify the exact react-reconciler ↔ react pairing from the registry, then write `packages/react/package.json`**

Run (network OK per constraints):
```bash
nix develop -c bash -c 'bun info react-reconciler@0.33.0 peerDependencies'
nix develop -c bash -c 'bun info react@19.2 version'
nix develop -c bash -c 'bun info @types/react-reconciler version 2>/dev/null || echo "no @types/react-reconciler — reconciler ships its own or use a manual d.ts"'
```
Expected: `peerDependencies` shows `react: ^19.2.0` (research-verified). Take the exact newest `react@19.2.x` `bun info` reports (e.g. `19.2.0`) and pin it. If the registry contradicts `^19.2.0`, pin the react version that satisfies the printed peer range and note the deviation in the commit body.

`packages/react/package.json` (fill the exact versions from the verify output; example values shown — **replace with what `bun info` prints**):
```json
{
  "name": "@nativedesktop/react",
  "version": "0.0.0",
  "type": "module",
  "main": "src/index.ts",
  "types": "src/index.ts",
  "exports": {
    ".": "./src/index.ts",
    "./jsx-runtime": "./src/jsx-runtime.ts"
  },
  "dependencies": {
    "react": "19.2.0",
    "react-reconciler": "0.33.0"
  },
  "devDependencies": {
    "@types/react": "19.2.0",
    "@types/react-reconciler": "0.32.0"
  }
}
```
Notes: (a) **no carets anywhere**. (b) `@types/react-reconciler` tracks reconciler minors loosely; pin the exact version `bun info @types/react-reconciler version` reports as compatible, or if none is published for 0.33 omit it and add a local `packages/react/src/react-reconciler.d.ts` shim (`declare module "react-reconciler"` + `declare module "react-reconciler/constants"`) — decide at implement-time based on the `bun info` result and note which path was taken. (c) `react-dom` is intentionally NOT a dependency — the JSX runtime types come from `@types/react`'s `react/jsx-runtime`, not react-dom.

- [ ] **Step 3: Write the package tsconfig and a stub barrel, then install**

`packages/react/tsconfig.json`:
```json
{
  "extends": "../../tsconfig.base.json",
  "compilerOptions": { "types": ["bun", "react"] },
  "include": ["src/**/*.ts", "src/**/*.tsx"]
}
```

`packages/react/src/index.ts` (stub — filled by Tasks 4–5):
```ts
export const version = "0.0.0";
```

Install (this creates and pins `bun.lock`):
```bash
nix develop -c bash -c 'bun install'
```
Expected: resolves `react@19.2.0` + `react-reconciler@0.33.0` exactly, writes `bun.lock`, populates `node_modules/`. Then confirm the reconciler and its constants module are importable:
```bash
nix develop -c bash -c 'cd packages/react && bun --print "typeof (await import(\"react-reconciler\")).default"'
nix develop -c bash -c 'cd packages/react && bun --print "Object.keys(await import(\"react-reconciler/constants\"))" | head'
```
Expected: `function`; a key list including `DiscreteEventPriority`, `ContinuousEventPriority`, `DefaultEventPriority`. If `react-reconciler/constants` is not a resolvable subpath in 0.33.0, fall back to importing the constants from the reconciler's own `Constants` export — verify with `bun --print "Object.keys(await import('react-reconciler'))"` and record the working import in Task 4.

- [ ] **Step 4: Type-check the empty package**

Run: `nix develop -c bash -c 'cd packages/react && bunx tsc --noEmit'`
Expected: clean. Fix the first error only (likely a missing `@types/react` or a `jsxImportSource` resolution issue — resolved by the `types: ["bun","react"]` override).

- [ ] **Step 5: Commit the scaffold + lockfile**
```bash
git add package.json tsconfig.base.json packages/react/package.json packages/react/tsconfig.json packages/react/src/index.ts bun.lock
git commit -m "feat: scaffold @nativedesktop/react workspace with pinned react 19.2 + reconciler 0.33"
```
Confirm `node_modules/` is NOT staged (it is gitignored): `git status --porcelain | rg node_modules` must print nothing.

### Task 4: The reconciler host config — pure render phase, batched commit phase, priority + Suspense

**Files:**
- Create: `packages/react/src/host-config.ts`
- Create: `packages/react/src/ids.ts` (generation-tagged id allocator)
- Create: `packages/react/src/ops.ts` (op-batch accumulator + node/handler registry)
- Create: `packages/react/src/renderer.ts` (reconciler instance + `render()` public API)
- Create: `packages/react/src/probe-hostconfig.ts` (dev-only: dumps required host-config keys — see Step 1)

**Interfaces:**
- Consumes: `../../runtime/ndp.ts` (`Ndp`, `Op`, `EventMsg`), `react-reconciler`, `react-reconciler/constants`.
- Produces:
  - `ids.ts`: `newGeneration(): void` (bumps the module generation counter) and `nextNodeId(): number` returning a generation-tagged id. Packing: `id = (generation << 24) | (seq++ & 0xFFFFFF)` with `seq` reset per generation — documented so the host can extract the generation for GC in a later milestone (M8). Both fit in a JS number and the u32 wire field.
  - `ops.ts`:
    - `class Batch { push(op: Op): void; drain(): Op[] }` — the per-commit accumulator.
    - `class NodeRegistry` keyed by node ID → `{ type, props, onClick?: () => void, textChildren?: string }`, with `register/get/update/unregister`.
  - `host-config.ts`: the full host config object (methods enumerated by the probe in Step 1), all render-phase methods pure, all commit-phase methods pushing to the active `Batch`.
  - `renderer.ts`:
    - `Reconciler = createReconciler(hostConfig)`.
    - `render(element: React.ReactNode): Promise<void>` — connects `Ndp`, handshakes, wires `onEvent` → dispatcher (seeds current update priority from the event's `priority`, looks up the node's `onClick`, calls it), creates the container, and calls `updateContainer`. `resetAfterCommit` flushes `batch.drain()` as one `ndp.sendCommit({ commitId, generation, ops })`.

- [ ] **Step 1: Dump the ACTUAL required host-config surface (do NOT trust blog posts)**

Write `packages/react/src/probe-hostconfig.ts`:
```ts
import Reconciler from "react-reconciler";
import * as React from "react";

const accessed = new Set<string>();
const base: Record<string, unknown> = {
  supportsMutation: true,
  supportsPersistence: false,
  supportsHydration: false,
  isPrimaryRenderer: true,
  noTimeout: -1,
  getRootHostContext: () => ({}),
  getChildHostContext: (c: unknown) => c,
  prepareForCommit: () => null,
  resetAfterCommit: () => {},
  clearContainer: () => {},
  createInstance: () => ({}),
  createTextInstance: () => ({}),
  appendInitialChild: () => {},
  finalizeInitialChildren: () => false,
  shouldSetTextContent: () => false,
  appendChild: () => {},
  appendChildToContainer: () => {},
  insertBefore: () => {},
  insertInContainerBefore: () => {},
  removeChild: () => {},
  removeChildFromContainer: () => {},
  commitUpdate: () => {},
  commitTextUpdate: () => {},
  hideInstance: () => {},
  unhideInstance: () => {},
  hideTextInstance: () => {},
  unhideTextInstance: () => {},
  getPublicInstance: (i: unknown) => i,
  detachDeletedInstance: () => {},
  maySuspendCommit: () => false,
  preloadInstance: () => true,
  startSuspendingCommit: () => {},
  suspendInstance: () => {},
  waitForCommitToBeReady: () => null,
  resolveUpdatePriority: () => 0,
  getCurrentUpdatePriority: () => 0,
  setCurrentUpdatePriority: () => {},
  shouldAttemptEagerTransition: () => false,
  trackSchedulerEvent: () => {},
  resolveEventType: () => null,
  resolveEventTimeStamp: () => -1.1,
  requestPostPaintCallback: () => {},
  scheduleMicrotask: (fn: () => void) => queueMicrotask(fn),
  supportsMicrotasks: true,
};
const proxy = new Proxy(base, {
  get(t, k: string) { accessed.add(k); return (t as Record<string, unknown>)[k] ?? (() => {}); },
});
const r = (Reconciler as unknown as (c: unknown) => { createContainer: (...a: unknown[]) => unknown; updateContainer: (...a: unknown[]) => void })(proxy);
const container = r.createContainer({}, 0, null, false, null, "nd", () => {}, () => {}, () => {}, null);
r.updateContainer(React.createElement("box", null, React.createElement("label", null, "hi")), container, null, () => {});
setTimeout(() => { console.log([...accessed].sort().join("\n")); }, 50);
```
Run: `nix develop -c bash -c 'cd packages/react && bun run src/probe-hostconfig.ts'`
This prints the exact set of keys react-reconciler 0.33.0 touched. **Implement `host-config.ts` to cover precisely that set** (superset the accessed keys with the priority/Suspense methods). If the probe reveals a method the table above missed (e.g. a renamed `getInstanceFromNode`), add it. The `createContainer`/`updateContainer` arities also come from this run — read `node_modules/react-reconciler/index.js` for the exact `createContainer` signature if the call throws on arity.

- [ ] **Step 2: Write `ids.ts` and `ops.ts`**

`packages/react/src/ids.ts`:
```ts
let generation = 0;
let seq = 0;

export function currentGeneration(): number {
  return generation;
}

export function newGeneration(): void {
  generation += 1;
  seq = 0;
}

/** Generation-tagged monotonic node id: (generation << 24) | (seq & 0xFFFFFF). */
export function nextNodeId(): number {
  seq += 1;
  return (generation << 24) | (seq & 0xffffff);
}
```

`packages/react/src/ops.ts`:
```ts
import type { Op } from "../../../runtime/ndp.ts";

export class Batch {
  private ops: Op[] = [];
  push(op: Op): void { this.ops.push(op); }
  drain(): Op[] { const o = this.ops; this.ops = []; return o; }
  get length(): number { return this.ops.length; }
}

export type Handler = () => void;

export interface NodeRecord {
  id: number;
  type: string;
  props: Record<string, unknown>;
  onClick?: Handler;
}

export class NodeRegistry {
  private byId = new Map<number, NodeRecord>();
  register(rec: NodeRecord): void { this.byId.set(rec.id, rec); }
  get(id: number): NodeRecord | undefined { return this.byId.get(id); }
  unregister(id: number): void { this.byId.delete(id); }
}
```

- [ ] **Step 3: Write `host-config.ts`**

The descriptor type for a host instance is `{ id: number; type: string; props: Record<string, unknown>; children: Instance[] }`. Render-phase methods build these in memory only. Commit-phase methods push ops to a `Batch` supplied via the container's `hostContext`/a module singleton (a module-scoped `activeBatch` set by the renderer before each commit is simplest and matches the single-threaded reconciler). Key method contracts:

```ts
import { DiscreteEventPriority, ContinuousEventPriority, DefaultEventPriority } from "react-reconciler/constants";
import type { Op } from "../../../runtime/ndp.ts";
import { nextNodeId } from "./ids.ts";
import { Batch, NodeRegistry } from "./ops.ts";

export type WidgetType = "window" | "box" | "label" | "button";
const WIDGET: Record<WidgetType, "Window" | "Box" | "Label" | "Button"> = {
  window: "Window", box: "Box", label: "Label", button: "Button",
};

export interface Instance {
  id: number;
  type: WidgetType;
  props: Record<string, unknown>;
}

// Set by the renderer immediately before updateContainer / on each commit.
export let activeBatch: Batch = new Batch();
export let registry: NodeRegistry = new NodeRegistry();
export function bindCommitTargets(b: Batch, r: NodeRegistry): void { activeBatch = b; registry = r; }

let currentUpdatePriority = DefaultEventPriority;

function textOf(children: unknown): string | undefined {
  if (typeof children === "string") return children;
  if (typeof children === "number") return String(children);
  if (Array.isArray(children) && children.every((c) => typeof c === "string" || typeof c === "number"))
    return children.join("");
  return undefined;
}

export const hostConfig = {
  supportsMutation: true,
  supportsPersistence: false,
  isPrimaryRenderer: true,
  noTimeout: -1 as const,
  supportsMicrotasks: true,
  scheduleMicrotask: queueMicrotask,

  getRootHostContext: () => ({ root: true }),      // non-null sentinel
  getChildHostContext: (parent: unknown) => parent, // non-null (parent is non-null)
  prepareForCommit: () => null,
  resetAfterCommit: () => {},                        // renderer overrides via wrapper; see renderer.ts
  clearContainer: () => {},

  // ---- render phase: PURE, no socket, no host widgets ----
  createInstance(type: WidgetType, props: Record<string, unknown>): Instance {
    const id = nextNodeId();
    const inst: Instance = { id, type, props };
    // create op is emitted at COMMIT via appendInitialChild? No — emit at createInstance
    // is a render-phase side effect and is FORBIDDEN. Emit `create` lazily on first
    // commit-time attach. Simplest correct approach: emit `create` here into a
    // deferred list keyed to the instance, flushed when the instance is first attached
    // in a commit method. See note below.
    return inst;
  },
  createTextInstance(text: string): { text: string } {
    return { text }; // folded into the parent label's text; see shouldSetTextContent
  },
  shouldSetTextContent: (type: WidgetType) => type === "label",
  appendInitialChild() {},        // pure: structure recorded at commit
  finalizeInitialChildren: () => false,

  // ---- commit phase: emit ops into activeBatch ----
  appendChild(parent: Instance, child: Instance) { emitCreateIfNew(child); activeBatch.push({ op: "append", parent: parent.id, child: child.id }); },
  appendChildToContainer(container: Container, child: Instance) { emitCreateIfNew(child); container.rootId = child.id; /* window */ },
  insertBefore(parent: Instance, child: Instance, before: Instance) { emitCreateIfNew(child); activeBatch.push({ op: "insertBefore", parent: parent.id, child: child.id, before: before.id }); },
  insertInContainerBefore(_c: Container, child: Instance, _b: Instance) { emitCreateIfNew(child); },
  removeChild(_parent: Instance, child: Instance) { activeBatch.push({ op: "remove", id: child.id }); registry.unregister(child.id); },
  removeChildFromContainer(_c: Container, child: Instance) { activeBatch.push({ op: "remove", id: child.id }); registry.unregister(child.id); },

  commitUpdate(inst: Instance, type: WidgetType, oldProps: Record<string, unknown>, newProps: Record<string, unknown>) {
    // React 19: no prepareUpdate — diff here.
    if (type === "label") {
      const t = textOf(newProps.children) ?? (newProps.text as string | undefined);
      const old = textOf(oldProps.children) ?? (oldProps.text as string | undefined);
      if (t !== undefined && t !== old) activeBatch.push({ op: "setText", id: inst.id, text: t });
    }
    const changed: Record<string, unknown> = {};
    for (const k of Object.keys(newProps)) {
      if (k === "children" || k === "onClick") continue;
      if (newProps[k] !== oldProps[k]) changed[k] = newProps[k];
    }
    if (Object.keys(changed).length) activeBatch.push({ op: "update", id: inst.id, props: changed });
    inst.props = newProps;
    // Re-register handler so events route to the latest closure.
    const rec = registry.get(inst.id);
    if (rec) rec.onClick = newProps.onClick as (() => void) | undefined;
  },
  commitTextUpdate() {}, // labels handle their own text via commitUpdate

  hideInstance(inst: Instance) { activeBatch.push({ op: "hide", id: inst.id }); },
  unhideInstance(inst: Instance) { activeBatch.push({ op: "unhide", id: inst.id }); },
  hideTextInstance() {},
  unhideTextInstance() {},

  getPublicInstance: (i: Instance) => i,
  detachDeletedInstance() {},
  maySuspendCommit: () => false,
  preloadInstance: () => true,
  startSuspendingCommit() {},
  suspendInstance() {},
  waitForCommitToBeReady: () => null,

  resolveUpdatePriority: () => currentUpdatePriority,
  getCurrentUpdatePriority: () => currentUpdatePriority,
  setCurrentUpdatePriority: (p: number) => { currentUpdatePriority = p; },
  shouldAttemptEagerTransition: () => false,
  trackSchedulerEvent() {},
  resolveEventType: () => null,
  resolveEventTimeStamp: () => -1.1,
  requestPostPaintCallback() {},
};

export interface Container { rootId: number | null }

function emitCreateIfNew(inst: Instance) {
  if (registry.get(inst.id)) return;
  const props = { ...inst.props };
  const text = textOf((props as Record<string, unknown>).children);
  if (inst.type === "label" && text !== undefined) props.text = text;
  delete (props as Record<string, unknown>).children;
  delete (props as Record<string, unknown>).onClick;
  activeBatch.push({ op: "create", id: inst.id, widget: WIDGET[inst.type], props });
  registry.register({ id: inst.id, type: inst.type, props: inst.props, onClick: inst.props.onClick as (() => void) | undefined });
}

export function setPriorityFor(kind: "discrete" | "continuous" | "default"): void {
  currentUpdatePriority = kind === "discrete" ? DiscreteEventPriority : kind === "continuous" ? ContinuousEventPriority : DefaultEventPriority;
}
```
**Critical design note (encode from the research gotchas):** `createInstance` must NOT emit a `create` op (concurrent render can discard the WIP tree → leaked host widgets). Instead a node's `create` op is emitted lazily by `emitCreateIfNew` the first time the node is *attached in a commit method* (`appendChild`/`insertBefore`/`appendChildToContainer`). This guarantees only committed nodes ever produce host widgets. The container's window node is the first attach target (`appendChildToContainer`). Confirm against the probe output that these are the exact attach methods 0.33.0 calls; if it also calls `appendChild` for the container, adjust.

- [ ] **Step 4: Write `renderer.ts`**

```ts
import ReconcilerFactory from "react-reconciler";
import { ConcurrentRoot } from "react-reconciler/constants";
import type { ReactNode } from "react";
import { Ndp, type EventMsg } from "../../../runtime/ndp.ts";
import { hostConfig, bindCommitTargets, setPriorityFor, type Container } from "./host-config.ts";
import { Batch, NodeRegistry } from "./ops.ts";
import { currentGeneration } from "./ids.ts";

export async function render(element: ReactNode): Promise<void> {
  const ndp = await Ndp.connect();
  await ndp.handshake({ name: "bun", version: Bun.version });

  const batch = new Batch();
  const registry = new NodeRegistry();
  bindCommitTargets(batch, registry);

  let commitId = 0;
  const configWithFlush = {
    ...hostConfig,
    resetAfterCommit() {
      const ops = batch.drain();
      if (ops.length) ndp.sendCommit({ commitId: commitId++, generation: currentGeneration(), ops });
    },
  };

  ndp.onEvent((e: EventMsg) => {
    setPriorityFor((e.priority as "discrete" | "continuous" | "default") ?? "discrete");
    const rec = registry.get(e.nodeId);
    if (e.name === "clicked") rec?.onClick?.();
  });

  const Reconciler = (ReconcilerFactory as unknown as (c: typeof configWithFlush) => any)(configWithFlush);
  const container: Container = { rootId: null };
  const root = Reconciler.createContainer(container, ConcurrentRoot, null, false, null, "nd", (e: unknown) => { throw e; }, () => {}, () => {}, null);
  Reconciler.updateContainer(element, root, null, () => {});

  // Keep the process alive so the reconciler's scheduler + event stream run.
  await new Promise<void>(() => {});
}
```
`ConcurrentRoot` must be the value the probe/`react-reconciler/constants` exports (verify: `bun --print "(await import('react-reconciler/constants')).ConcurrentRoot"`; if it is not there, pass the numeric root tag the reconciler expects — read `createContainer`'s signature). The `createContainer` arity is confirmed by the probe in Step 1; adjust argument count to match 0.33.0 exactly.

- [ ] **Step 5: Type-check**

Run: `nix develop -c bash -c 'cd packages/react && bunx tsc --noEmit'`
Expected: clean. The reconciler default export is loosely typed; the `as unknown as (...)` casts localize the untyped boundary. Fix the first error only.

- [ ] **Step 6: Commit**
```bash
git add packages/react/src/host-config.ts packages/react/src/ids.ts packages/react/src/ops.ts packages/react/src/renderer.ts packages/react/src/probe-hostconfig.ts
git commit -m "feat: mutation-mode react-reconciler host config with batched commits and priority wiring"
```

### Task 5: JSX intrinsic types + jsx-runtime re-export + package barrel

**Files:**
- Create: `packages/react/src/jsx.d.ts` (global `JSX.IntrinsicElements`)
- Create: `packages/react/src/jsx-runtime.ts` (re-export react's jsx-runtime so the package can serve as `jsxImportSource` if desired; also lets the demo import from `@nativedesktop/react/jsx-runtime`)
- Modify: `packages/react/src/index.ts` (barrel: export `render`, types)

**Interfaces:**
- Consumes: `@types/react` (for `ReactNode`/key/children typing).
- Produces:
  - `JSX.IntrinsicElements` with `window`/`box`/`label`/`button` and the M2 prop subsets.
  - `packages/react/src/index.ts` exporting `render` (from `renderer.ts`) and the public types.

- [ ] **Step 1: Write `packages/react/src/jsx.d.ts`**

```ts
import type { ReactNode } from "react";

declare global {
  namespace JSX {
    interface IntrinsicElements {
      window: { title?: string; defaultWidth?: number; defaultHeight?: number; children?: ReactNode };
      box: { orientation?: "vertical" | "horizontal"; spacing?: number; children?: ReactNode };
      label: { text?: string; children?: ReactNode };
      button: { label?: string; onClick?: () => void; children?: ReactNode };
    }
  }
}

export {};
```
Prop subsets are exactly M2's: `Window {title, defaultWidth, defaultHeight}`, `Box {orientation, spacing}`, `Label {text}` (plus text children per the mapping), `Button {label, onClick}`.

- [ ] **Step 2: Write `packages/react/src/jsx-runtime.ts` and the barrel**

`packages/react/src/jsx-runtime.ts`:
```ts
export { jsx, jsxs, Fragment } from "react/jsx-runtime";
```
With `tsconfig.base.json`'s `jsxImportSource: "react"`, the demo's `.tsx` already resolves `react/jsx-runtime`; this file simply makes `@nativedesktop/react/jsx-runtime` a valid alias if a consumer sets `jsxImportSource: "@nativedesktop/react"`. Keep `jsxImportSource: "react"` in the demo (simplest); the alias exists for parity with the spec's "the package owns JSX".

`packages/react/src/index.ts`:
```ts
import "./jsx.d.ts";
export { render } from "./renderer.ts";
export type { Instance } from "./host-config.ts";
```

- [ ] **Step 3: Type-check**

Run: `nix develop -c bash -c 'cd packages/react && bunx tsc --noEmit'`
Expected: clean; `JSX.IntrinsicElements` now recognizes the four lowercase tags. Fix the first error only.

- [ ] **Step 4: Commit**
```bash
git add packages/react/src/jsx.d.ts packages/react/src/jsx-runtime.ts packages/react/src/index.ts
git commit -m "feat: typed jsx intrinsics and public barrel for @nativedesktop/react"
```

### Task 6: The counter demo app (`examples/counter`)

**Files:**
- Create: `examples/counter/package.json`
- Create: `examples/counter/tsconfig.json`
- Create: `examples/counter/main.tsx`

**Interfaces:**
- Consumes: `@nativedesktop/react` (`render` + JSX intrinsics) via `workspace:*`.
- Produces: an app importing ONLY `@nativedesktop/react` and `react` that renders the counter tree with `useState`, `Suspense`, and `startTransition`, plus a CI-visible interval commit and a distinctive Suspense-resolution commit.

- [ ] **Step 1: Write `examples/counter/package.json` and tsconfig**

`examples/counter/package.json`:
```json
{
  "name": "counter-example",
  "private": true,
  "type": "module",
  "dependencies": {
    "@nativedesktop/react": "workspace:*",
    "react": "19.2.0"
  }
}
```
(`react` is listed for the app's own `useState`/`Suspense`/`use` imports; the exact same pinned version as the package so bun dedupes to one copy — verify after `bun install` that only one `react` resolves: `bun pm ls | rg react`.)

`examples/counter/tsconfig.json`:
```json
{
  "extends": "../../tsconfig.base.json",
  "compilerOptions": { "types": ["bun", "react"] },
  "include": ["**/*.ts", "**/*.tsx"]
}
```

- [ ] **Step 2: Write `examples/counter/main.tsx`**

```tsx
import { render } from "@nativedesktop/react";
import { Suspense, use, useState, useTransition, useMemo } from "react";

// A promise that resolves after ~1s, memoized so it isn't recreated each render.
function useOneShot<T>(value: T, ms: number): Promise<T> {
  return useMemo(() => new Promise<T>((r) => setTimeout(() => r(value), ms)), [value, ms]);
}

function DelayedBadge(): React.ReactNode {
  const promise = useOneShot("ready:suspense-resolved", 1000);
  const text = use(promise); // suspends until resolved -> fallback shown, then unhidden
  return <label text={text} />;
}

function App(): React.ReactNode {
  const [clicks, setClicks] = useState(0);
  const [uptime, setUptime] = useState(0);
  const [slow, setSlow] = useState("idle");
  const [, startTransition] = useTransition();

  // Uptime interval keeps ND_COMMIT_APPLIED flowing under headless CI (no input synthesis).
  // useMemo runs the setup once; the interval drives state so commits continue.
  useMemo(() => {
    setInterval(() => setUptime((s) => s + 1), 500);
  }, []);

  const onClick = (): void => {
    setClicks((c) => c + 1);                       // discrete lane
    startTransition(() => setSlow(`transition:${Date.now()}`)); // transition lane
  };

  return (
    <window title="NativeDesktop M3 Counter" defaultWidth={480} defaultHeight={320}>
      <box orientation="vertical" spacing={8}>
        <label text={`Clicks: ${clicks}`} />
        <button label="Increment" onClick={onClick} />
        <label text={`Uptime: ${uptime}s`} />
        <label text={`Slow: ${slow}`} />
        <Suspense fallback={<label text="loading..." />}>
          <DelayedBadge />
        </Suspense>
      </box>
    </window>
  );
}

await render(<App />);
```
Notes: (a) the `DelayedBadge` label's resolved text `ready:suspense-resolved` is the **distinctive Suspense-resolution commit** the headless script greps for (via the `ND_UNHIDE` marker AND/OR the `setText`— see Task 8). (b) `useMemo(() => setInterval(...), [])` is the interval-setup idiom (the repo's no-raw-useEffect policy applies to `apps/web`/`apps/admin` only, not this Bun example, but this keeps the demo effect-free anyway). (c) top-level `await render(...)` matches the M2 demo's top-level-await style; bun supports it.

- [ ] **Step 3: Install the new workspace member and type-check**

Run:
```bash
nix develop -c bash -c 'bun install'
nix develop -c bash -c 'cd examples/counter && bunx tsc --noEmit'
```
Expected: `bun install` links `@nativedesktop/react` into `examples/counter` (workspace symlink); tsc clean with the JSX intrinsics recognized. If tsc cannot find the intrinsics, ensure `examples/counter/tsconfig.json` picks them up — the `import { render } from "@nativedesktop/react"` pulls `index.ts` which `import "./jsx.d.ts"` registers the global augmentation; if the global augmentation does not cross the package boundary, add `"types"` or a triple-slash `/// <reference types="@nativedesktop/react" />` at the top of `main.tsx` and note it. Fix the first error only.

- [ ] **Step 4: Commit**
```bash
git add examples/counter/package.json examples/counter/tsconfig.json examples/counter/main.tsx bun.lock
git commit -m "feat: interactive counter demo with usestate, suspense, and starttransition"
```

### Task 7: End-to-end run against the live host (interactive or headless dry-run)

**Files:** none created; wires the pieces and proves a real React commit reaches GTK.

**Interfaces:**
- Consumes: Task 1 host binary, Task 4–6 renderer + demo.
- Produces: evidence that `ND_SCRIPT=examples/counter/main.tsx zig build run` produces `ND_HELLO_OK`, a create-heavy first `ND_COMMIT_APPLIED`, repeating uptime commits, and (after ~1s) an `ND_UNHIDE` + `setText` for the resolved Suspense label.

- [ ] **Step 1: Run the host with the counter script**

Run:
```bash
nix develop -c bash -c 'zig build && NDP_TRACE=1 ND_SCRIPT=examples/counter/main.tsx timeout 4 ./zig-out/bin/nd-hello 2>&1 | tee /tmp/m3-run.log | tail -40'
```
Expected in the log: `ND_CHILD_CONNECTED`, `>> {"type":"hello"...}`, `<< {"type":"helloAck"...}`, `ND_HELLO_OK`, then a first `commitBatch` containing `create` ops for window/box/labels/button + `append`/`insertBefore`, `ND_COMMIT_APPLIED commitId=0`; repeating uptime `setText` commits every 500ms; and around commitId reflecting ~1s later, a `hide`→(fallback shown)→`unhide` sequence and a `setText ... ready:suspense-resolved` for the `DelayedBadge` label. Confirm no `ND_WARN unknown op` lines appear (means every emitted op is handled).

If the reconciler throws at `createContainer`/`updateContainer` arity, fix `renderer.ts` against the probe output from Task 4 Step 1 (this is the one integration seam most likely to need adjustment) and re-run. One hypothesis at a time.

- [ ] **Step 2: Grep the exact Suspense-resolution observable**

Run:
```bash
grep -E "ND_UNHIDE|ready:suspense-resolved" /tmp/m3-run.log
```
Expected: at least one `ND_UNHIDE id=<n>` line (fallback hidden then real content unhidden) OR the `setText` frame carrying `ready:suspense-resolved`. This is the observable Task 8's headless script asserts. Record which one fires reliably (prefer `ND_UNHIDE` if Suspense hides the fallback; if 0.33 instead removes/re-adds the fallback, the observable is the `remove` of the fallback + `create` of the resolved label — capture whichever the trace actually shows and wire that grep into Task 8).

- [ ] **Step 3: No commit** (verification only). If `renderer.ts` needed an arity fix, fold it into a `fix:` commit:
```bash
git add packages/react/src/renderer.ts
git commit -m "fix: match react-reconciler 0.33 createContainer arity"
```

### Task 8: `scripts/headless-m3.sh` + CI wiring

**Files:**
- Create: `scripts/headless-m3.sh`
- Modify: `.github/workflows/ci.yml` (append one step)

**Interfaces:**
- Consumes: Task 1 host, Task 6 demo, weston (M1 flake).
- Produces: `scripts/headless-m3.sh` exits 0 iff, under a headless compositor with a **unique** weston socket, the host prints `ND_HELLO_OK`, ≥3 `ND_COMMIT_APPLIED`, and the Suspense-resolution observable from Task 7 Step 2. CI runs exactly this script.

- [ ] **Step 1: Write `scripts/headless-m3.sh`** (mirror `scripts/headless-m2.sh`; unique socket name `nd-headless-m3`)

```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
export WAYLAND_DISPLAY=nd-headless-m3
export GSK_RENDERER=cairo
export GDK_BACKEND=wayland
export ND_SCRIPT=examples/counter/main.tsx

weston --backend=headless --socket="$WAYLAND_DISPLAY" --idle-time=0 &
WESTON_PID=$!
trap 'kill "$WESTON_PID" 2>/dev/null || true' EXIT

for _ in $(seq 1 50); do
  [ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ] && break
  sleep 0.1
done

# ~3s covers handshake + several 500ms uptime commits + the ~1s Suspense resolution.
OUT=$(NDP_TRACE=1 timeout 4 ./zig-out/bin/nd-hello 2>&1 || true)
echo "$OUT"

grep -q "ND_HELLO_OK" <<<"$OUT" || { echo "FAIL: no handshake"; exit 1; }
COMMITS=$(grep -c "ND_COMMIT_APPLIED" <<<"$OUT" || true)
[ "$COMMITS" -ge 3 ] || { echo "FAIL: only $COMMITS commits applied"; exit 1; }
# Suspense resolution observable (see Task 7 Step 2 — use whichever the trace reliably shows).
grep -Eq "ND_UNHIDE|ready:suspense-resolved" <<<"$OUT" || { echo "FAIL: suspense did not resolve"; exit 1; }
echo "headless m3: OK ($COMMITS commits, suspense resolved)"
```
`ND_SCRIPT` is exported so the host (`src/runtime.zig:63`, `parent_env.get("ND_SCRIPT")`) spawns `bun examples/counter/main.tsx`. `NDP_TRACE=1` is set so the `ready:suspense-resolved` grep can match the `setText` frame text even if the `ND_UNHIDE` marker path differs — belt and suspenders. If Task 7 established that only one of the two observables fires, keep both in the `grep -E` (an OR match is robust to which one the reconciler emits).

- [ ] **Step 2: Run it — expect failure before build/install, then success**

Run: `nix develop -c bash -c 'chmod +x scripts/headless-m3.sh && bun install && zig build && ./scripts/headless-m3.sh'`
Expected: markers stream, then `headless m3: OK (N commits, suspense resolved)` with N ≥ 3, exit 0. If it fails on `Cannot find module @nativedesktop/react`, run `bun install` at the repo root first (workspace link). Fix the first failure only.

- [ ] **Step 3: Extend `.github/workflows/ci.yml`**

The file already has `unit tests` / `build` / `headless smoke` / `headless m2` / `kill -9`. Add a `bun install` step (needed so the workspace + lockfile are materialized in CI before the m3 run) and the `headless m3` step. Insert after the `kill -9 crash isolation` step:
```yaml
      - name: bun install (workspace)
        run: nix develop -c bun install --frozen-lockfile
      - name: headless m3
        run: nix develop -c ./scripts/headless-m3.sh
```
`--frozen-lockfile` enforces the committed `bun.lock` (fails if it drifted — the CI guard on the exact pins). Do not rewrite the file — only these two `- name:` blocks are added.

- [ ] **Step 4: Validate the full CI sequence locally**

Run:
```bash
nix develop -c bash -c 'zig build test && zig build && ./scripts/headless-smoke.sh && ./scripts/headless-m2.sh && ./scripts/kill9-test.sh && bun install --frozen-lockfile && ./scripts/headless-m3.sh'
```
Expected: all green — the exact sequence CI runs. Also run the TS type-check gate that CI could add later: `nix develop -c bash -c 'cd packages/react && bunx tsc --noEmit && cd ../../examples/counter && bunx tsc --noEmit'`.

- [ ] **Step 5: Commit**
```bash
git add scripts/headless-m3.sh .github/workflows/ci.yml
git commit -m "ci: headless m3 job running the react counter demo end to end"
```

### Task 9: Binary command-buffer encoding SPEC DOCUMENT (spec D4) — schema only, no implementation

**Files:**
- Create: `docs/superpowers/specs/2026-07-09-ndp-binary-encoding.md`

**Interfaces:**
- Consumes: the JSON op list (spec §4; `src/protocol.zig` `Op`) — the binary encoding is **1:1** with it.
- Produces: a schema document (no code) describing the fixed-layout opcode stream from D4: `opcode u8`, generation-tagged `u32` node IDs, interned string table. Shipped as a spec; implementation is deferred to M10 behind the 10k-node benchmark.

- [ ] **Step 1: Write the spec document**

`docs/superpowers/specs/2026-07-09-ndp-binary-encoding.md` must contain, at minimum:
- **Status/provenance header** matching the design-spec style (Date 2026-07-09; Status: schema, not implemented; references spec §4, D4, and this M3 plan).
- **Framing.** Same `u32 LE length ‖ payload` outer frame as JSON (§4); the payload is either UTF-8 JSON (encoding `"json"`, default) or the binary command buffer (encoding `"binary"`, negotiated via the existing `helloAck.encodings` capability list — no new handshake). The tracer (`NDP_TRACE`) decodes binary back to the identical JSON text (spec §4 "greppability survives the binary migration").
- **CommitBatch layout.** Header: `magic u8` / `version u8` / `commitId u64 LE` / `generation u32 LE` / `opCount u32 LE` / `stringTableOffset u32 LE`, followed by the op stream, followed by the interned string table.
- **Op stream.** Each op: `opcode u8` then a fixed field layout per opcode. Opcode table 1:1 with the JSON ops — assign stable numbers:

  | opcode | op | fields |
  |---|---|---|
  | `0x01` | `create` | `id u32`, `widgetType u16` (enum), `propCount u16`, then `propCount × (keyRef u32, valueTag u8, value…)` |
  | `0x02` | `append` | `parent u32`, `child u32` |
  | `0x03` | `insertBefore` | `parent u32`, `child u32`, `before u32` (0 = head/none) |
  | `0x04` | `remove` | `id u32` |
  | `0x05` | `setText` | `id u32`, `textRef u32` |
  | `0x06` | `update` | `id u32`, `propCount u16`, then props as in `create` |
  | `0x07` | `hide` | `id u32` |
  | `0x08` | `unhide` | `id u32` |

- **Node IDs.** `u32`, generation-tagged: high 8 bits generation, low 24 bits sequence (`(generation << 24) | seq`) — **must match the renderer's `ids.ts` packing** (cross-reference this M3 plan Task 4). Document that `0` is reserved as "no node" (used by `insertBefore.before`).
- **Interned string table.** All strings (prop keys, string prop values, `setText` text, widget-type names if not enum'd) are interned once per batch; ops reference them by `u32` index into the table. Table layout: `count u32 LE`, then `count × (len u32 LE, UTF-8 bytes)`. Rationale: 10k-node mounts repeat prop keys (`"title"`, `"orientation"`) thousands of times; interning is the main win over JSON.
- **Value tags.** `valueTag u8`: `0x00 null`, `0x01 bool(u8)`, `0x02 i64 LE`, `0x03 f64 LE`, `0x04 stringRef(u32)`. Covers the JSON prop value space (props are `std.json.Value` today).
- **Widget-type enum.** `u16` map `Window=1, Box=2, Label=3, Button=4, …` — extended by the M5 widget schema codegen; document that the enum is generated from `widgets.schema.json` (spec §6) so JSON and binary share one source of truth.
- **Explicit non-goals for this document.** No encoder/decoder code, no benchmark (both are M10). This document is the contract the M10 implementation and the JSON path must both satisfy; a golden-vector appendix (one `CommitBatch` shown in both JSON and annotated hex) SHOULD be included so M10 has a test fixture.
- **Golden vector appendix.** Include one worked example: the counter demo's first commit (window+box+two labels+button) rendered as annotated hex, byte-offset-labeled, matching its JSON form. This doubles as the M10 conformance fixture.

- [ ] **Step 2: Self-check the document is 1:1 with the JSON ops**

Cross-reference every op in `src/protocol.zig`'s `Op` (after Task 1: create/append/insertBefore/remove/setText/update/hide/unhide) against the opcode table — all eight must appear, none extra. Verify: `rg -n '"(create|append|insertBefore|remove|setText|update|hide|unhide)"' src/tree.zig` lists exactly the eight the doc encodes.

- [ ] **Step 3: Commit**
```bash
git add docs/superpowers/specs/2026-07-09-ndp-binary-encoding.md
git commit -m "docs: ndp binary command-buffer encoding schema (d4, m3 deliverable)"
```

---

## Self-review notes

**M3 scope coverage (spec §14 M3 line + owner contract, not expanded):**
- `@nativedesktop/react` bundling react@19.2.x + react-reconciler@0.33.0 in mutation mode — Task 3 (exact pins, `bun.lock` committed, `peerDependency react ^19.2.0` verified), Task 4 (`supportsMutation:true` host config).
- Descriptor-only render phase — Task 4 (`createInstance`/`createTextInstance`/`appendInitialChild`/`finalizeInitialChildren` pure; `create` op emitted lazily at first commit-time attach via `emitCreateIfNew`, never in render — directly encodes the research gotcha that concurrent render discards WIP trees and must not leak host widgets).
- One CommitBatch per React commit — Task 4 (`resetAfterCommit` drains the `Batch` into a single `sendCommit`).
- Generation-tagged node IDs — Task 4 `ids.ts` (`(generation << 24) | seq`), mirrored in the binary-encoding spec (Task 9).
- hide/unhide + update-priority wired — Task 4 (`hideInstance`/`unhideInstance` → `hide`/`unhide` ops; `resolveUpdatePriority`/`get`/`set` against `react-reconciler/constants`; event dispatcher seeds priority).
- Interactive TSX counter with useState, Suspense fallback, startTransition against live GTK — Task 6 (`main.tsx`), Task 7 (live-host verification), Task 8 (headless assertion).
- Host-side ops remove/insertBefore/hide/unhide (spec §4 verbatim) — Task 1 (protocol golden tests + tree arms + gtk_backend via verified `setVisible`/`Box.remove`/`Box.insertChildAfter`), `runtime/ndp.ts` `Op` extended.
- Binary command-buffer SPEC DOCUMENT (D4, schema only) — Task 9.

**Architecture constraints honored:** two processes unchanged (D1); one closure per commit via the landed `marshalCommit`→`invokeFull`→`tree.apply` path (no runtime.zig change — the new ops are new `tree.apply` arms); render-phase purity enforced by emitting `create` only at commit-time attach; priorities from `react-reconciler/constants`; `getRootHostContext`/`getChildHostContext` non-null sentinels; `commitUpdate` self-diffs (no `prepareUpdate`); JSON transport (binary is D4/M10 doc only); Suspense hide/unhide mapped to `gtk_widget_set_visible` (the exact gotcha the research flagged: fallbacks otherwise leave widgets permanently hidden). The required host-config surface is **discovered by a Proxy probe against the installed 0.33.0** (Task 4 Step 1), not trusted from memory — directly implementing the research's "run the reconciler and log missing methods rather than trusting blog posts" instruction.

**Landed-code fidelity:** every reference to host internals matches the landed files, not the M2 plan text — `Runtime.start`'s five-arg signature, `std.Io.Mutex`, `ND_SCRIPT` default and `bun <script>` spawn, `tree.apply`'s if-chain + `ND_COMMIT_APPLIED` marker, the permissive `Op` struct, the single `gtk_imports` array in `build.zig`, and `runtime/ndp.ts`'s exact `Ndp` API (`connect`/`handshake`/`sendCommit`/`onEvent`/`ping`). New markers `ND_HIDE`/`ND_UNHIDE`/`ND_REMOVE` are emitted in `tree.apply` exactly as the headless-m3 script greps for them.

**Parallelization:** the header note marks the Zig track (Tasks 1–2) and renderer track (Tasks 3–7) as file-disjoint and concurrently runnable; Task 8 integrates and must run last; Task 9 (doc) is standalone. Within the renderer track, Task 4 depends on Task 3, Task 5 is parallel to Task 4, Tasks 6–7 depend on 4+5.

**Where an npm/registry fact must be confirmed at implement-time, the exact command is embedded** rather than guessed: `bun info react-reconciler@0.33.0 peerDependencies`, `bun info react@19.2 version`, `bun info @types/react-reconciler version` (Task 3 Step 2); the Proxy host-config probe (`bun run src/probe-hostconfig.ts`) for the authoritative method set + `createContainer` arity (Task 4 Step 1); `bun pm ls | rg react` for single-copy dedupe (Task 6); GTK symbol `rg` re-verifications (Task 1 Step 1). The plan's example version strings (`19.2.0`, `0.33.0`, `@types/react` `19.2.0`) are placeholders to **replace with the verify output**.

**Judgment calls made within the constraints:**
1. **Text mapping = Label `text` prop via `shouldSetTextContent`.** Chosen over inventing a standalone GTK text node (no backing widget) — `shouldSetTextContent("label")` returns true so React folds a label's string/number children into host text; the renderer emits `setText`. The counter's changing count flows `useState`→`commitUpdate`→`setText`. (Owner explicitly left this to the plan; documented in the constraints block and Task 4.)
2. **`create` op emitted lazily at first commit-time attach**, not in `createInstance`. This is the only correct way to honor "render phase is pure / real ops only at commit" while still assigning IDs during render — IDs are pure (just counter increments); the op is deferred to `emitCreateIfNew` called from `appendChild`/`insertBefore`/`appendChildToContainer`. Directly prevents the concurrent-render widget leak from the research.
3. **Workspace mechanism = root `package.json` with `workspaces: ["packages/*","examples/*"]` + `workspace:*` dep**, `runtime/ndp.ts` imported by relative path (`../../runtime/ndp.ts`) not re-exported. Bun's idiomatic monorepo mechanism; `runtime/` stays put per the constraint; the transport is framework-internal, not a public entrypoint.
4. **Shared `tsconfig.base.json` + per-package `extends`** rather than one root tsconfig — matches the existing `runtime/tsconfig.json` prior art and lets `packages/react` set `jsxImportSource` without forcing it on `runtime/`.
5. **No `build.zig` change** — the new ops live in `tree.zig`/`gtk_backend.zig` (compiled transitively via `main.zig`) and the golden tests live in `protocol.zig` (already a `protocol_tests` target). Nothing new to wire.
6. **Suspense observable = OR of `ND_UNHIDE` and the `ready:suspense-resolved` setText text** (Task 8), with the live run (Task 7 Step 2) determining which the reconciler reliably emits; the OR grep is robust to 0.33's choice of hide-vs-remove for fallbacks.
7. **Host-config surface discovered by a Proxy probe** (Task 4 Step 1) rather than hardcoded from the research table — the research itself says the interface must be re-verified at every React version and to log missing methods rather than trust docs. The table in the conventions block is a starting superset; the probe is authoritative.
8. **`react-dom` is NOT a dependency** — JSX runtime types come from `@types/react`'s `react/jsx-runtime`; adding react-dom would pull an unused, conflicting renderer.
9. **`--frozen-lockfile` in CI** (Task 8 Step 3) makes the committed `bun.lock` and the exact pins a hard CI gate — drift fails the build, satisfying "exact-pinned, bun.lock committed".
```
