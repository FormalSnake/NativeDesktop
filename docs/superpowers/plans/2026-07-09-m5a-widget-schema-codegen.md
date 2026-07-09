# M5a — Widget Schema + Codegen Foundation: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Parallelism note (read before dispatching waves).** M5a has **two tracks that both touch `src/`, so they are NOT fully parallel.** Sequence:
> - **Track A — schema + codegen + generated outputs (Tasks 1–4).** Authors `schema/widgets.json`, `tools/codegen.ts`, and commits the generated TS (`packages/react/src/generated/*`), Zig (`src/generated/widgets.zig`), docs (`docs/widgets.md`), and rewires `jsx-runtime.ts` + `gtk_backend.zig` to consume them. Touches `schema/`, `tools/`, `packages/react/src/`, `src/gtk_backend.zig`, `docs/`, `build.zig`.
> - **Track B — null backend + conformance (Tasks 5–6).** Adds `src/backend.zig` (the backend-interface seam), `src/null_backend.zig`, `src/conformance.zig`, and wires the seam into `src/tree.zig` + `src/gtk_backend.zig`. Touches `src/backend.zig`, `src/null_backend.zig`, `src/conformance.zig`, `src/tree.zig`, `src/gtk_backend.zig`, `build.zig`.
> **Both tracks edit `src/gtk_backend.zig` and `build.zig`, so run Track A fully before Track B.** Track A's Task 3 converts `gtk_backend.zig` into a thin dispatcher over the generated table; Track B's Task 5 then wraps that dispatcher behind the backend-interface struct. Doing B first would rebase-conflict the exact same functions. **Task 7 (CI freshness + full-gate integration) runs last.** Within Track A, Tasks are strictly sequential (2 depends on 1, 3 depends on 2, 4 depends on 3). Within Track B, Task 6 depends on Task 5.

**Goal:** Land the codegen foundation that spec §6 and decision **D6** mandate: **one machine-readable widget schema is the contract; hand-written per-widget bindings are banned.** From the M4 host (React drives GTK, automation layer live), M5a replaces the four hand-written widget appliers with **code generated from `schema/widgets.json`** by a single Bun script (`tools/codegen.ts`), emitting: (a) the TS/JSX intrinsic prop types, (b) the Zig per-widget create + prop-applier functions, (c) a generated widget reference doc, and (d) the automation role/`textFrom` tables. Generated files are committed with a `// GENERATED` header and **byte-stable ordering**, and CI re-runs codegen + `git diff --exit-code` to prove freshness. In parallel, M5a lands the **null backend** (an in-memory backend recording ops into inspectable state) plus a **schema-driven conformance suite** (`src/conformance.zig`) that pins schema semantics with plain `zig build test` — no display server. The schema v1 content is **exactly the four existing widgets** (Window, Box, Label, Button); the format anticipates the other ~16 (containers/children rules, enums) but adding them is M5b.

**Architecture:** Unchanged two-process topology (D1). The change is entirely in *how the four widget bindings come to exist*: today `src/gtk_backend.zig` hand-writes `createWidget`/`applyProps`/`setText` with an `if (eql(kind,"Window")) … else if …` ladder; after M5a those bodies are **generated** into `src/generated/widgets.zig` from the schema, and `gtk_backend.zig` becomes a thin dispatcher that forwards to the generated table (preserving its existing public function signatures so `tree.zig` and `automation.zig` need no changes). Identically, `packages/react/src/jsx-runtime.ts` stops hand-declaring `JSX.IntrinsicElements` and instead re-exports the generated `packages/react/src/generated/intrinsics.ts`. The **backend seam** for the null backend is introduced as a Zig **comptime-selected backend module** (not a runtime C vtable — spec §7's C vtable is a later milestone): `tree.zig` and the generated widget code call through a `Backend` abstraction whose concrete implementation (`gtk_backend` vs `null_backend`) is chosen at test-target-build time. The conformance test target builds `tree`/generated-widgets against `null_backend`; the real exe builds them against `gtk_backend` exactly as today.

**Tech Stack:** Zig 0.16.0 (exact), the vendored zig-gobject bindings at `vendor/gobject-bindings` (glib2/gobject2/gio2/gtk4 + the M4-added gsk4/gdk4/graphene1), Bun 1.3.13 (flake devshell) for `tools/codegen.ts` — **no npm deps beyond the workspace** (uses only `Bun.file`/`import.meta.dir` and standard `JSON`). TypeScript strict; `bunx tsc --noEmit`. GTK4 ≥ 4.20 (devshell 4.22.4). Conformance runs under plain `zig build test` (no weston).

## Global Constraints

Carried over from M1–M4, unchanged:

- Zig is exactly `0.16.0`; `build.zig`'s `checkZigVersion()` guard stays.
- Bun is pinned `1.3.13` (flake devshell). `tools/codegen.ts` must run under it with **zero added dependencies**.
- No `@cImport` anywhere; all GTK access goes through the vendored zig-gobject modules imported as `glib`/`gobject`/`gio`/`gtk`. **The generated Zig introduces no new GTK symbols** — it re-emits the exact calls `gtk_backend.zig` already makes (verified-symbol table below is the closed set).
- **No hand-written per-widget bindings (D6).** After M5a, the four widget appliers exist **only** as codegen output. Editing `src/generated/widgets.zig` or `packages/react/src/generated/*` by hand is a policy violation — the fix is to edit `schema/widgets.json` and/or `tools/codegen.ts` and regenerate.
- Generated files carry a first-line header `// GENERATED by tools/codegen.ts — do not edit` (or `//` → `/* */`-free `//`-comment equivalent for TS) and are **committed** (deterministic, byte-stable output).
- Headless CI uses `weston --backend=headless` + `GSK_RENDERER=cairo` for the existing scripts; conformance adds **no** display-server dependency (`zig build test` only).
- Commit style: short imperative lowercase subject (e.g. `feat: widget schema + codegen`). No co-author trailers, no body unless closing an issue.
- All commands run inside the devshell (direnv activates it; in CI, `nix develop -c`).
- Host prints machine-greppable markers to **stderr**. M5a adds none to the runtime; behavior is byte-identical to M4.
- `git add` **explicit paths per task** — never `git add -A`. Confirm `node_modules/` is never staged.

### M5a-new constraints (owner decisions, verbatim)

- **Schema is the single source of truth.** `schema/widgets.json` is versioned (`"schemaVersion": 1`). It covers **exactly** Window, Box, Label, Button and their **current** props (including `testID` on all four), each prop typed (`{"type": "string"|"int"|"bool"|"enum", ...}` with optional `"values"` for enums, `"required"?`, `"default"?`), events (Button: `clicked`), a `container` section for widgets that accept children, and an `automation` section per widget (`role`, `textFrom` = which prop supplies the readable text, or `null`). Adding the other ~16 widgets is **M5b** — but the schema *format* must already express: enums (`orientation`), container/children rules (Window = single child, Box = multi child), and a place for events. Do not add widgets not currently implemented.
- **Codegen emits four artifacts, deterministically.** `tools/codegen.ts` reads `schema/widgets.json` and writes, with **byte-stable ordering** (iterate widgets and props in schema-declaration order, never `Object.keys` hash order — the schema arrays *are* the order):
  1. `packages/react/src/generated/intrinsics.ts` — the `JSX.IntrinsicElements` interface + the `WidgetType`/`WidgetName` unions, replacing the hand-written types in `jsx-runtime.ts`.
  2. `src/generated/widgets.zig` — `pub fn create(...)` and `pub fn applyProps(...)` dispatch tables plus per-widget create/apply bodies, against the vendored `gtk`/`glib` modules, replacing the hand-written bodies in `gtk_backend.zig`.
  3. `docs/widgets.md` — a generated widget reference (one section per widget: props table, events, automation role/textFrom).
  4. `packages/react/src/generated/schema-meta.ts` — role/`textFrom` tables keyed by NDP widget name, consumed later by automation/MCP (M5b/M6). Exported as a typed const.
- **`jsx-runtime.ts` becomes a re-export.** Its own `JSX.IntrinsicElements` declaration is deleted; it re-exports from `./generated/intrinsics.ts`. The `jsxImportSource=@nativedesktop/react` resolution (the reason this lives in-package, not a `declare global`) is preserved — the generated file declares the `JSX` namespace, `jsx-runtime.ts` re-exports it.
- **`gtk_backend.zig` becomes a thin dispatcher.** Its **public function signatures are unchanged** (`createWidget`, `connectButtonClick`, `appendChild`, `setText`, `removeChild`, `insertBefore`, `setVisible`, `applyProps`, `getWindow`, `setEventSink`) so `tree.zig` and `automation.zig` compile untouched. The bodies of `createWidget`/`applyProps` **delegate** to `@import("generated/widgets.zig")`. The container/text/visibility ops (`appendChild`/`setText`/`removeChild`/`insertBefore`/`setVisible`) and the window-tracking/`the_window` logic and `connectButtonClick`/`onClicked` signal plumbing and the arena/`dupeZ` helpers **stay in `gtk_backend.zig`** (they are backend-structural, not per-widget property appliers — codegen owns only the per-widget create/prop-apply bodies).
- **Generated Zig must be byte-behavior-identical to M4.** The generated `create`/`applyProps` must produce the *same GTK calls in the same order with the same defaults* as the current hand-written code (Window: `ApplicationWindow.new`→setTitle→setDefaultSize(480,320)→present; Box: orientation default vertical, spacing default 0; Label: text default ""; Button: label default "Button"; applyProps Box.spacing + Window.title). The M4 gate (counter demo, headless scripts) is the regression proof.
- **Null backend + conformance.** `src/null_backend.zig` is an in-memory backend implementing the **same create/applyProps/append/insertBefore/remove/setText/setVisible surface** as `gtk_backend`, but recording each operation into inspectable in-memory state (a per-node record of type, applied props, children order, text, visibility). `src/conformance.zig` is a `zig build test`-run target that drives a **schema-embedded** suite: for every widget in the schema, create-with-defaults then apply each declared prop and assert the recorded state matches; for containers, exercise append/insertBefore/remove and assert child ordering. This pins schema semantics with **no display server**.
- **Backend seam = comptime module selection (pick ONE, justified).** Introduce `src/backend.zig` exposing a `pub const Backend` that is a **struct of the backend's public functions** resolved at comptime from a build option. Concretely: `build.zig` passes a build option `backend` (`"gtk"` default, `"null"` for the conformance target); `backend.zig` does `pub const impl = if (opt == .null) @import("null_backend.zig") else @import("gtk_backend.zig");` and re-exports its functions. `tree.zig` and the generated widget code call `backend.impl.<fn>` instead of `@import("gtk_backend.zig").<fn>`. **Justification:** a comptime module swap is the minimal seam that (1) lets conformance build the *real* `tree.apply` logic against an in-memory backend with **zero runtime cost and no vtable**, (2) leaves the shipping exe byte-identical (same comptime branch as today), and (3) does not pre-commit the C-ABI vtable shape that spec §7 defers to M6+. A runtime function-pointer vtable would add an indirection the shipping path doesn't need and would duplicate what the C vtable milestone will design properly; comptime selection is the smaller, reversible diff.
- **The generated Zig is backend-agnostic where it can be, GTK-typed where it must.** The per-widget create bodies call GTK constructors returning `*gtk.Widget` — these are inherently GTK-typed. Therefore **`src/generated/widgets.zig` is the GTK code generator's output and is imported by `gtk_backend.zig` only**; the null backend does **not** import it (the null backend hand-implements the same surface in ~80 lines, recording ops — it is deliberately tiny and not code-generated in M5a; generating N backends' bodies from the schema is an M6 concern once the C vtable exists). The conformance suite therefore validates *the schema's declared semantics* (defaults, prop set, container rules) against the null backend's recording — the GTK generated code is validated by the M4 gate (real widgets on real GTK).
- **CI freshness gate.** A new CI step runs `nix develop -c bun tools/codegen.ts` then `git diff --exit-code -- schema packages/react/src/generated src/generated docs/widgets.md` — a stale checked-in generated file fails CI. Document the invocation (`bun tools/codegen.ts`, the future `nd codegen`) in the plan and wire it as a CI step before the existing build step.

### Landed-code reality (authoritative — read before writing)

| Fact | Landed reality (file:line) |
|---|---|
| Hand-written widget create ladder to REPLACE | `src/gtk_backend.zig:44-72` (`createWidget` if/else over Window/Box/Label/Button). |
| Hand-written applyProps to REPLACE | `src/gtk_backend.zig:132-144` (`applyProps` Box.spacing + Window.title). |
| Structural ops that STAY in gtk_backend | `appendChild` (85), `setText` (96), `removeChild` (101), `insertBefore` (112), `setVisible` (128), `connectButtonClick`/`onClicked` (75/80), arena/`dupeZ` (6/20), `propStr`/`propInt` (24/34), `the_window`/`getWindow`/`setEventSink` (8/16/12). |
| Hand-written JSX intrinsics to REPLACE | `packages/react/src/jsx-runtime.ts:12-23` (`JSX.IntrinsicElements` window/box/label/button + `testID` on each). |
| `jsx-runtime.ts` also re-exports the runtime | `export { jsx, jsxs, Fragment } from "react/jsx-runtime";` (`jsx-runtime.ts:3`) — **keep this line**, only the `namespace JSX` block moves to generated. |
| Host-side `WidgetType` union (JS) | `packages/react/src/host-config.ts:5-8` maps lowercase intrinsic → NDP name (`window`→`Window` …). **Not schema-driven in M5a** — leave as-is; only `jsx-runtime.ts` consumes generated types. (Noting it so implementers don't duplicate.) |
| `tree.apply` calls into backend | `src/tree.zig:120,133,137,141,147,151,157,161` all call `backend.<fn>` where `backend = @import("gtk_backend.zig")` (`tree.zig:4`). Task 5 repoints this import to `@import("backend.zig").impl`. |
| Backend import in automation | `src/automation.zig:10` `const backend = @import("gtk_backend.zig")` — uses only `getWindow()`. Task 5 leaves this pointing at `gtk_backend` (automation is GTK-only; it never runs against null). |
| build.zig test targets | `build.zig:42-70`: `tests` (main.zig, full gtk imports), `protocol_tests`, `tree_tests`. Task 6 adds a `conformance_tests` target built with the `backend=null` option; Task 5 adds the `backend` build option + `src/backend.zig`. |
| build.zig gtk_imports array | `build.zig:16-24` — reused by exe + tests. The conformance target needs **no** gtk imports if `null_backend` is pure-std; confirm `null_backend.zig` imports no `gtk`. |
| props type on the wire | `op.props: ?std.json.Value` (`src/protocol.zig:42`); `propStr`/`propInt` extract typed values (`gtk_backend.zig:24,34`). Generated `applyProps` reuses these helpers. |
| CI is linear | `.github/workflows/ci.yml` ends at `headless m4` (line 28). Task 7 inserts a `codegen freshness` step **before** `build` and re-verifies the tail. |
| Full gate command | `nix develop -c bash -c 'zig build test && zig build && bun install --frozen-lockfile && ./scripts/headless-smoke.sh && ./scripts/headless-m2.sh && ./scripts/kill9-test.sh && ./scripts/headless-m3.sh && ./scripts/headless-m4.sh'` (CLAUDE-activeContext.md). |

### Verified-symbol table (re-verify inside the devshell before pasting)

**No new GTK symbols are introduced by M5a** — the generated Zig re-emits the closed set already used in `src/gtk_backend.zig`. Confirmed present this session in `vendor/gobject-bindings/src/gtk4/gtk4.zig`; line numbers are from this session, re-run the `rg` if they drift — the **symbol name** is the contract.

| Need | Symbol (Zig binding) | Verify command → expected |
|---|---|---|
| App window ctor | `gtk.ApplicationWindow.new(app) *gtk.ApplicationWindow` | `rg -n "gtk_application_window_new\b" vendor/gobject-bindings/src/gtk4/gtk4.zig` → `2388` |
| Window title | `gtk.Window.setTitle(win, [*:0]const u8)` | `rg -n "gtk_window_set_title\b" vendor/gobject-bindings/src/gtk4/gtk4.zig` → present |
| Window default size | `gtk.Window.setDefaultSize(win, c_int, c_int)` | `rg -n "gtk_window_set_default_size\b" vendor/gobject-bindings/src/gtk4/gtk4.zig` → present |
| Window present | `gtk.Window.present(win)` | `rg -n "gtk_window_present\b" vendor/gobject-bindings/src/gtk4/gtk4.zig` → present |
| Box ctor | `gtk.Box.new(gtk.Orientation, c_int) *gtk.Box` | `rg -n "gtk_box_new\b" vendor/gobject-bindings/src/gtk4/gtk4.zig` → `3268` |
| Box spacing | `gtk.Box.setSpacing(box, c_int)` | `rg -n "gtk_box_set_spacing\b" vendor/gobject-bindings/src/gtk4/gtk4.zig` → `3344` |
| Label ctor | `gtk.Label.new(?[*:0]const u8) *gtk.Label` | `rg -n "gtk_label_new\b" vendor/gobject-bindings/src/gtk4/gtk4.zig` → `27215` |
| Label text | `gtk.Label.setText(label, [*:0]const u8)` | `rg -n "gtk_label_set_text\b" vendor/gobject-bindings/src/gtk4/gtk4.zig` → `27557` |
| Button ctor | `gtk.Button.newWithLabel([*:0]const u8) *gtk.Button` | `rg -n "gtk_button_new_with_label\b" vendor/gobject-bindings/src/gtk4/gtk4.zig` → `4501` |
| Button label (applyProps future) | `gtk.Button.setLabel(button, [*:0]const u8)` | `rg -n "gtk_button_set_label\b" vendor/gobject-bindings/src/gtk4/gtk4.zig` → `4584` |
| `.as(gtk.Widget)` upcast | `widget.as(gtk.Widget)` (gobject cast helper, used throughout `gtk_backend.zig`) | already used at `src/gtk_backend.zig:54,60,64,68` |

Zig std idioms (0.16, verified this session, already used in-repo):

| Need | Symbol / idiom | Where used today |
|---|---|---|
| Unmanaged growable list | `var xs: std.ArrayList(T) = .empty;` then `try xs.append(alloc, item)` | `src/automation.zig:334,339` |
| Testing allocator + asserts | `std.testing.allocator`, `expectEqualStrings`, `expectEqual`, `expect` | `src/tree.zig:172-181`, `src/protocol.zig` tests |
| Parse JSON to `std.json.Value` | `std.json.parseFromSlice(std.json.Value, gpa, bytes, .{})` | `src/protocol.zig:120` (struct form); conformance uses the `Value` form |
| Build option | `b.option(...)` + `std.Build.Module.addOptions`/`root_module.addImport` of an options module | new in Task 5 — mirror standard `build.zig` option idiom (see Task 5 code) |

---

## Schema format (the contract — `schema/widgets.json` v1)

The complete v1 file content is in **Task 1** below (paste verbatim). Format rules the codegen and M5b rely on:

- Top-level: `{"schemaVersion": 1, "widgets": [ … ]}`. `widgets` is an **ordered array** (declaration order = codegen emit order = byte stability).
- Each widget: `{"name": "<NDPName>", "intrinsic": "<lowercaseTag>", "props": [ … ], "events": [ … ], "container": {…}|null, "automation": {…}}`.
  - `name` — the NDP wire name (`"Window"`); `intrinsic` — the JSX tag (`"window"`).
  - `props` — **ordered array** of `{"name","type","tsType"?,"required"?,"default"?,"values"?,"appliesTo"?}`.
    - `type` ∈ `"string" | "int" | "bool" | "enum"`. For `"enum"`, `values` is a non-empty ordered string array and `tsType` is omitted (codegen builds the union from `values`).
    - `default` — the value the create body uses when the prop is absent (Window `defaultWidth` 480, Box `spacing` 0, Label `text` "", Button `label` "Button").
    - `appliesTo` ∈ `"create" | "createAndUpdate" | "meta"`. `"meta"` props (only `testID`) are **never** applied to the widget — they flow to `NodeMeta` host-side (M4 behavior) and are typed in TS but excluded from the Zig applier. `"createAndUpdate"` props (Box `spacing`, Window `title`) are emitted in **both** the create body and `applyProps`. `"create"` props are only in the create body.
  - `events` — ordered array of `{"name","ndpName"?}` (Button `clicked` → JS `onClick`; `ndpName` documents the JS handler name).
  - `container` — `null` for leaves (Label, Button); for containers, `{"childModel": "single" | "multi"}` (Window = `single`, Box = `multi`). Drives conformance's append/insertBefore/remove ordering tests.
  - `automation` — `{"role": "<string>", "textFrom": "<propName>" | null}`. `role` is the semantic role for `getTree` (M5b/M6); `textFrom` names the prop that supplies readable text (Label `text`, Button `label`, Window `title`, Box `null`).

---

## TASK 1 — Author `schema/widgets.json` (v1: the four widgets)

**Track A. Depends on: nothing. Files: `schema/widgets.json`.**

- [ ] Create `schema/widgets.json` with **exactly** this content (byte-for-byte; the declaration order here fixes generated output order):

```json
{
  "schemaVersion": 1,
  "widgets": [
    {
      "name": "Window",
      "intrinsic": "window",
      "container": { "childModel": "single" },
      "props": [
        { "name": "title", "type": "string", "appliesTo": "createAndUpdate" },
        { "name": "defaultWidth", "type": "int", "default": 480, "appliesTo": "create" },
        { "name": "defaultHeight", "type": "int", "default": 320, "appliesTo": "create" },
        { "name": "testID", "type": "string", "appliesTo": "meta" }
      ],
      "events": [],
      "automation": { "role": "window", "textFrom": "title" }
    },
    {
      "name": "Box",
      "intrinsic": "box",
      "container": { "childModel": "multi" },
      "props": [
        { "name": "orientation", "type": "enum", "values": ["vertical", "horizontal"], "default": "vertical", "appliesTo": "create" },
        { "name": "spacing", "type": "int", "default": 0, "appliesTo": "createAndUpdate" },
        { "name": "testID", "type": "string", "appliesTo": "meta" }
      ],
      "events": [],
      "automation": { "role": "group", "textFrom": null }
    },
    {
      "name": "Label",
      "intrinsic": "label",
      "container": null,
      "props": [
        { "name": "text", "type": "string", "default": "", "appliesTo": "create" },
        { "name": "testID", "type": "string", "appliesTo": "meta" }
      ],
      "events": [],
      "automation": { "role": "label", "textFrom": "text" }
    },
    {
      "name": "Button",
      "intrinsic": "button",
      "container": null,
      "props": [
        { "name": "label", "type": "string", "default": "Button", "appliesTo": "create" },
        { "name": "testID", "type": "string", "appliesTo": "meta" }
      ],
      "events": [
        { "name": "clicked", "ndpName": "onClick" }
      ],
      "automation": { "role": "button", "textFrom": "label" }
    }
  ]
}
```

**Commit:**
```bash
git add schema/widgets.json
git commit -m "feat: versioned widget schema (v1: window/box/label/button)"
```

**Verify:**
```bash
nix develop -c bun -e 'const s=await Bun.file("schema/widgets.json").json(); if(s.schemaVersion!==1)throw new Error("bad version"); if(s.widgets.map(w=>w.name).join(",")!=="Window,Box,Label,Button")throw new Error("bad order/set"); console.log("schema ok:",s.widgets.length,"widgets")'
```
Expected: `schema ok: 4 widgets`.

### Interfaces (produced by this task)
- `schema/widgets.json` — the single source of truth consumed by `tools/codegen.ts` (Task 2) and embedded by conformance (Task 6, via `@embedFile`).

---

## TASK 2 — Author `tools/codegen.ts` (the generator) + first generation

**Track A. Depends on: Task 1. Files: `tools/codegen.ts`, and the four generated outputs it writes.**

- [ ] Create `tools/codegen.ts` with **exactly** this content. It reads `schema/widgets.json` relative to the repo root (derived from `import.meta.dir`), and writes the four artifacts. It uses **only** Bun/standard APIs (no imports).

```ts
#!/usr/bin/env bun
// tools/codegen.ts — reads schema/widgets.json and emits the generated
// TS/JSX intrinsics, Zig widget appliers, docs, and automation meta tables.
// Deterministic: widgets and props are emitted in schema-declaration order.
// Run: `bun tools/codegen.ts`. Never hand-edit the generated files.

import { resolve } from "node:path";

const ROOT = resolve(import.meta.dir, "..");
const HEADER_TS = "// GENERATED by tools/codegen.ts — do not edit\n";
const HEADER_ZIG = "// GENERATED by tools/codegen.ts — do not edit\n";

type PropType = "string" | "int" | "bool" | "enum";
type AppliesTo = "create" | "createAndUpdate" | "meta";

interface Prop {
  name: string;
  type: PropType;
  values?: string[];
  default?: string | number | boolean;
  required?: boolean;
  appliesTo: AppliesTo;
}
interface Event { name: string; ndpName?: string }
interface Widget {
  name: string;
  intrinsic: string;
  container: { childModel: "single" | "multi" } | null;
  props: Prop[];
  events: Event[];
  automation: { role: string; textFrom: string | null };
}
interface Schema { schemaVersion: number; widgets: Widget[] }

function tsTypeOf(p: Prop): string {
  switch (p.type) {
    case "string": return "string";
    case "int": return "number";
    case "bool": return "boolean";
    case "enum": return (p.values ?? []).map((v) => JSON.stringify(v)).join(" | ");
  }
}

function zigLit(p: Prop): string {
  if (p.type === "int") return String(p.default ?? 0);
  if (p.type === "bool") return String(p.default ?? false);
  return JSON.stringify(String(p.default ?? "")); // string/enum default as a Zig string literal
}

// ---- artifact (a): TS/JSX intrinsics ----
function genIntrinsics(s: Schema): string {
  let out = HEADER_TS;
  out += 'import type { ReactNode } from "react";\n\n';
  out += "export { jsx, jsxs, Fragment } from \"react/jsx-runtime\";\n\n";
  out += "export type WidgetName = " + s.widgets.map((w) => JSON.stringify(w.name)).join(" | ") + ";\n";
  out += "export type WidgetType = " + s.widgets.map((w) => JSON.stringify(w.intrinsic)).join(" | ") + ";\n\n";
  out += "export namespace JSX {\n";
  out += "  export interface IntrinsicElements {\n";
  for (const w of s.widgets) {
    const fields: string[] = [];
    for (const p of w.props) fields.push(`${p.name}?: ${tsTypeOf(p)}`);
    for (const e of w.events) fields.push(`${e.ndpName ?? e.name}?: () => void`);
    fields.push("children?: ReactNode");
    out += `    ${w.intrinsic}: { ${fields.join("; ")} };\n`;
  }
  out += "  }\n";
  out += "  export type Element = ReactNode;\n";
  out += "  export interface ElementChildrenAttribute {\n    children: {};\n  }\n";
  out += "}\n";
  return out;
}

// ---- artifact (d): automation role/textFrom meta ----
function genSchemaMeta(s: Schema): string {
  let out = HEADER_TS;
  out += "export interface WidgetMeta {\n  role: string;\n  textFrom: string | null;\n  childModel: \"single\" | \"multi\" | null;\n}\n\n";
  out += "export const widgetMeta: Record<string, WidgetMeta> = {\n";
  for (const w of s.widgets) {
    const cm = w.container ? JSON.stringify(w.container.childModel) : "null";
    out += `  ${JSON.stringify(w.name)}: { role: ${JSON.stringify(w.automation.role)}, textFrom: ${w.automation.textFrom === null ? "null" : JSON.stringify(w.automation.textFrom)}, childModel: ${cm} },\n`;
  }
  out += "};\n";
  return out;
}

// ---- artifact (b): Zig widget appliers ----
function genZig(s: Schema): string {
  let out = HEADER_ZIG;
  out += "const std = @import(\"std\");\n";
  out += "const gtk = @import(\"gtk\");\n\n";
  out += "// Extracts a typed prop from the on-wire `?std.json.Value`. Mirrors the\n";
  out += "// helpers that used to live inline in gtk_backend.zig.\n";
  out += "fn propStr(props: ?std.json.Value, key: []const u8) ?[]const u8 {\n";
  out += "    const v = props orelse return null;\n";
  out += "    if (v != .object) return null;\n";
  out += "    const field = v.object.get(key) orelse return null;\n";
  out += "    return switch (field) {\n        .string => field.string,\n        else => null,\n    };\n}\n\n";
  out += "fn propInt(props: ?std.json.Value, key: []const u8) ?i64 {\n";
  out += "    const v = props orelse return null;\n";
  out += "    if (v != .object) return null;\n";
  out += "    const field = v.object.get(key) orelse return null;\n";
  out += "    return switch (field) {\n        .integer => field.integer,\n        else => null,\n    };\n}\n\n";
  out += "/// The GTK create dispatcher. `dupeZ` turns a wire string into a NUL-\n";
  out += "/// terminated GTK string using the backend arena (passed by the caller).\n";
  out += "pub fn create(\n";
  out += "    app: *gtk.Application,\n";
  out += "    kind: []const u8,\n";
  out += "    props: ?std.json.Value,\n";
  out += "    dupeZ: *const fn ([]const u8) [:0]const u8,\n";
  out += "    the_window: *?*gtk.Window,\n";
  out += ") !*gtk.Widget {\n";
  for (let i = 0; i < s.widgets.length; i++) {
    const w = s.widgets[i];
    const kw = i === 0 ? "if" : "} else if";
    out += `    ${kw} (std.mem.eql(u8, kind, ${JSON.stringify(w.name)})) {\n`;
    out += genZigCreateBody(w);
  }
  out += "    }\n";
  out += "    std.debug.print(\"ND_WARN unknown widget kind={s}\\n\", .{kind});\n";
  out += "    return error.UnknownWidget;\n";
  out += "}\n\n";
  out += "/// The GTK update dispatcher for createAndUpdate props.\n";
  out += "pub fn applyProps(widget: *gtk.Widget, kind: []const u8, props: ?std.json.Value, dupeZ: *const fn ([]const u8) [:0]const u8) void {\n";
  let firstApply = true;
  for (const w of s.widgets) {
    const updProps = w.props.filter((p) => p.appliesTo === "createAndUpdate");
    if (updProps.length === 0) continue;
    const kw = firstApply ? "if" : "} else if";
    firstApply = false;
    out += `    ${kw} (std.mem.eql(u8, kind, ${JSON.stringify(w.name)})) {\n`;
    out += genZigApplyBody(w, updProps);
  }
  if (!firstApply) out += "    }\n";
  out += "}\n";
  return out;
}

function genZigCreateBody(w: Widget): string {
  let out = "";
  if (w.name === "Window") {
    out += "        const window = gtk.ApplicationWindow.new(app);\n";
    out += "        const win = window.as(gtk.Window);\n";
    out += "        the_window.* = win;\n";
    out += "        if (propStr(props, \"title\")) |t| gtk.Window.setTitle(win, dupeZ(t));\n";
    out += `        const w: c_int = @intCast(propInt(props, "defaultWidth") orelse ${dflt(w, "defaultWidth")});\n`;
    out += `        const h: c_int = @intCast(propInt(props, "defaultHeight") orelse ${dflt(w, "defaultHeight")});\n`;
    out += "        gtk.Window.setDefaultSize(win, w, h);\n";
    out += "        gtk.Window.present(win);\n";
    out += "        return window.as(gtk.Widget);\n";
  } else if (w.name === "Box") {
    out += "        const vertical = if (propStr(props, \"orientation\")) |o| std.mem.eql(u8, o, \"vertical\") else true;\n";
    out += "        const orientation: gtk.Orientation = if (vertical) .vertical else .horizontal;\n";
    out += `        const spacing: c_int = @intCast(propInt(props, "spacing") orelse ${dflt(w, "spacing")});\n`;
    out += "        const box = gtk.Box.new(orientation, spacing);\n";
    out += "        return box.as(gtk.Widget);\n";
  } else if (w.name === "Label") {
    out += `        const text = propStr(props, "text") orelse ${zigDefaultStr(w, "text")};\n`;
    out += "        const label = gtk.Label.new(dupeZ(text));\n";
    out += "        return label.as(gtk.Widget);\n";
  } else if (w.name === "Button") {
    out += `        const lbl = propStr(props, "label") orelse ${zigDefaultStr(w, "label")};\n`;
    out += "        const button = gtk.Button.newWithLabel(dupeZ(lbl));\n";
    out += "        return button.as(gtk.Widget);\n";
  } else {
    throw new Error(`no create template for widget ${w.name} — add one when introducing it (M5b)`);
  }
  return out;
}

function genZigApplyBody(w: Widget, updProps: Prop[]): string {
  let out = "";
  for (const p of updProps) {
    if (w.name === "Box" && p.name === "spacing") {
      out += "        if (propInt(props, \"spacing\")) |s| {\n";
      out += "            const box: *gtk.Box = @ptrCast(@alignCast(widget));\n";
      out += "            gtk.Box.setSpacing(box, @intCast(s));\n";
      out += "        }\n";
    } else if (w.name === "Window" && p.name === "title") {
      out += "        if (propStr(props, \"title\")) |t| {\n";
      out += "            const win: *gtk.Window = @ptrCast(@alignCast(widget));\n";
      out += "            gtk.Window.setTitle(win, dupeZ(t));\n";
      out += "        }\n";
    } else {
      throw new Error(`no applyProps template for ${w.name}.${p.name} — add one when introducing it (M5b)`);
    }
  }
  return out;
}

function dflt(w: Widget, prop: string): string {
  const p = w.props.find((x) => x.name === prop)!;
  return String(p.default);
}
function zigDefaultStr(w: Widget, prop: string): string {
  const p = w.props.find((x) => x.name === prop)!;
  return JSON.stringify(String(p.default ?? ""));
}

// ---- artifact (c): docs ----
function genDocs(s: Schema): string {
  let out = "<!-- GENERATED by tools/codegen.ts — do not edit -->\n\n";
  out += "# Widget reference\n\n";
  out += `Schema version: ${s.schemaVersion}. Generated from \`schema/widgets.json\`.\n\n`;
  for (const w of s.widgets) {
    out += `## ${w.name} (\`<${w.intrinsic}>\`)\n\n`;
    out += `Automation role: \`${w.automation.role}\`. `;
    out += `Text source: ${w.automation.textFrom ? "`" + w.automation.textFrom + "`" : "none"}. `;
    out += `Children: ${w.container ? w.container.childModel : "none"}.\n\n`;
    out += "| Prop | Type | Default | Applied |\n|---|---|---|---|\n";
    for (const p of w.props) {
      const type = p.type === "enum" ? (p.values ?? []).join(" \\| ") : p.type;
      const def = p.default === undefined ? "—" : String(p.default);
      out += `| \`${p.name}\` | ${type} | ${def} | ${p.appliesTo} |\n`;
    }
    out += "\n";
    if (w.events.length) {
      out += "Events: " + w.events.map((e) => `\`${e.name}\` (\`${e.ndpName ?? e.name}\`)`).join(", ") + ".\n\n";
    }
  }
  return out;
}

async function writeIfChanged(rel: string, content: string): Promise<void> {
  const path = resolve(ROOT, rel);
  await Bun.write(path, content);
  console.log("wrote", rel, `(${content.length} bytes)`);
}

const schema = (await Bun.file(resolve(ROOT, "schema/widgets.json")).json()) as Schema;
await writeIfChanged("packages/react/src/generated/intrinsics.ts", genIntrinsics(schema));
await writeIfChanged("packages/react/src/generated/schema-meta.ts", genSchemaMeta(schema));
await writeIfChanged("src/generated/widgets.zig", genZig(schema));
await writeIfChanged("docs/widgets.md", genDocs(schema));
console.log("codegen complete");
```

- [ ] Run it once to produce the initial generated files:
```bash
nix develop -c bun tools/codegen.ts
```
Expected (order matters):
```
wrote packages/react/src/generated/intrinsics.ts (...)
wrote packages/react/src/generated/schema-meta.ts (...)
wrote src/generated/widgets.zig (...)
wrote docs/widgets.md (...)
codegen complete
```

- [ ] Confirm determinism — running twice produces no diff:
```bash
nix develop -c bash -c 'bun tools/codegen.ts >/dev/null && git add -N schema/widgets.json packages/react/src/generated src/generated docs/widgets.md 2>/dev/null; bun tools/codegen.ts >/dev/null && git diff --stat -- packages/react/src/generated src/generated docs/widgets.md'
```
Expected: no output (byte-stable).

- [ ] Confirm the generated Zig `create`/`applyProps` bodies match the M4 hand-written logic. Eyeball `src/generated/widgets.zig` against `src/gtk_backend.zig:44-72` and `:132-144` — same GTK calls, same defaults, same order.

**Commit:**
```bash
git add tools/codegen.ts packages/react/src/generated/intrinsics.ts packages/react/src/generated/schema-meta.ts src/generated/widgets.zig docs/widgets.md
git commit -m "feat: codegen for widget intrinsics, appliers, docs, meta"
```

### Interfaces (produced by this task)
- `tools/codegen.ts` — `bun tools/codegen.ts` regenerates all four artifacts idempotently.
- `packages/react/src/generated/intrinsics.ts` — exports `JSX` namespace, `WidgetName`, `WidgetType`, and re-exports `jsx`/`jsxs`/`Fragment`.
- `packages/react/src/generated/schema-meta.ts` — exports `widgetMeta: Record<string, {role, textFrom, childModel}>`.
- `src/generated/widgets.zig` — `pub fn create(app, kind, props, dupeZ, the_window) !*gtk.Widget` and `pub fn applyProps(widget, kind, props, dupeZ) void`.
- `docs/widgets.md` — generated reference.

---

## TASK 3 — Rewire `jsx-runtime.ts` and `gtk_backend.zig` onto the generated code

**Track A. Depends on: Task 2. Files: `packages/react/src/jsx-runtime.ts`, `src/gtk_backend.zig`.**

- [ ] Replace `packages/react/src/jsx-runtime.ts` with a thin re-export (delete its hand-written `namespace JSX`; keep pointing `jsxImportSource` at this module):

```ts
// The JSX intrinsic types and the jsx/jsxs/Fragment runtime re-exports are
// GENERATED from schema/widgets.json — see tools/codegen.ts. This module is a
// stable re-export point so jsxImportSource=@nativedesktop/react keeps
// resolving JSX.IntrinsicElements from the package (not from @types/react,
// whose HTML tag names would collide).
export { jsx, jsxs, Fragment, JSX } from "./generated/intrinsics.ts";
```

- [ ] Rewrite `src/gtk_backend.zig` so `createWidget` and `applyProps` delegate to the generated module, keeping **all other functions and helpers byte-identical**. Concretely:
  - Add `const generated = @import("generated/widgets.zig");` near the top imports.
  - Replace the `createWidget` body (`:44-72`) with a delegation that preserves the existing signature and the button-click connection is still done by `tree.zig` (unchanged — `tree.zig` calls `connectButtonClick` after `createWidget`):
    ```zig
    pub fn createWidget(app: *gtk.Application, kind: []const u8, props: ?std.json.Value) !*gtk.Widget {
        return generated.create(app, kind, props, &dupeZ, &the_window);
    }
    ```
  - Replace the `applyProps` body (`:132-144`) with:
    ```zig
    pub fn applyProps(widget: *gtk.Widget, kind: []const u8, props: ?std.json.Value) void {
        generated.applyProps(widget, kind, props, &dupeZ);
    }
    ```
  - `dupeZ` currently returns `[:0]const u8` and takes `[]const u8` (`:20`) — it already matches the `*const fn ([]const u8) [:0]const u8` pointer type the generated code expects. Take its address as `&dupeZ`.
  - `the_window` is `var the_window: ?*gtk.Window = null;` (`:8`) — pass `&the_window` so the generated Window arm can set it. Delete the now-duplicated `propStr`/`propInt` helpers from `gtk_backend.zig` **only if** nothing else in the file uses them; `createWidget`/`applyProps` were their only callers, so remove them (they live in the generated file now). **Verify with `rg -n "propStr|propInt" src/gtk_backend.zig` before deleting — if any structural op uses them, keep them.**

- [ ] `bunx tsc --noEmit` for the react package and build Zig:
```bash
nix develop -c bash -c 'cd packages/react && bunx tsc --noEmit'
nix develop -c zig build
```
Expected: both succeed with no errors.

- [ ] Run the react/counter typecheck path (the counter imports intrinsics transitively):
```bash
nix develop -c bash -c 'cd examples/counter && bunx tsc --noEmit'
```
Expected: no errors — the counter's `<window>`/`<box>`/`<label>`/`<button>` still typecheck against the generated intrinsics.

**Commit:**
```bash
git add packages/react/src/jsx-runtime.ts src/gtk_backend.zig
git commit -m "refactor: consume generated intrinsics + widget appliers"
```

### Interfaces (unchanged public surface — the point of this task)
- `gtk_backend.createWidget`/`applyProps` signatures identical to M4; `tree.zig`/`automation.zig` untouched.
- `@nativedesktop/react/jsx-runtime` still exports `JSX`/`jsx`/`jsxs`/`Fragment`.

---

## TASK 4 — Verify Track A regression (counter runs unchanged on GTK)

**Track A. Depends on: Task 3. Files: none (verification only).**

- [ ] Run the display-server-dependent gate tail to prove byte-identical runtime behavior:
```bash
nix develop -c bash -c 'zig build test && zig build && bun install --frozen-lockfile && ./scripts/headless-m3.sh && ./scripts/headless-m4.sh'
```
Expected: `headless-m3.sh` asserts `ND_UNHIDE|ready:suspense-resolved`; `headless-m4.sh` drives the counter (`getTree`, click ×3, `waitFor Clicks: 3`, screenshot) — both pass exactly as in M4. The generated appliers produce identical GTK calls, so nothing observable changes.

- [ ] If either script fails, the generated Zig diverged from the hand-written logic. Diff `src/generated/widgets.zig` against the M4 `git show HEAD~N:src/gtk_backend.zig` create/applyProps arms and reconcile **in `tools/codegen.ts`** (never hand-edit the generated file), then re-run Task 2's generation + this task.

**No new commit** (verification task). If a codegen fix was needed, amend it into the Task 2 codegen commit or add a `fix: …` commit touching only `tools/codegen.ts` + regenerated files.

---

## TASK 5 — Backend seam (`src/backend.zig`) + null backend (`src/null_backend.zig`)

**Track B. Depends on: Task 3 (gtk_backend is now the dispatcher). Files: `src/backend.zig`, `src/null_backend.zig`, `src/tree.zig`, `build.zig`.**

- [ ] Add a `backend` build option and an options module in `build.zig`. Mirror the standard `b.option` + options-module idiom. Add near the top of `build()` (after `optimize`):
```zig
const backend_kind = b.option([]const u8, "backend", "widget backend: gtk|null") orelse "gtk";
const build_opts = b.addOptions();
build_opts.addOption([]const u8, "backend", backend_kind);
```
- [ ] Create `src/backend.zig` — the comptime seam. It imports the build options and re-exports the selected backend's public functions:
```zig
const build_options = @import("build_options");

pub const impl = if (std.mem.eql(u8, build_options.backend, "null"))
    @import("null_backend.zig")
else
    @import("gtk_backend.zig");

const std = @import("std");
```
- [ ] Repoint `src/tree.zig`'s backend import (`:4`) from `@import("gtk_backend.zig")` to `@import("backend.zig").impl`. Every `backend.<fn>` call in `tree.apply` now resolves to the selected backend. **`automation.zig` keeps importing `gtk_backend.zig` directly** (it is GTK-only; never built against null).
- [ ] Wire `build_options` into the modules that import `backend.zig`. Both the main exe module and the `tree_tests` module need the options import added to their `imports` (alongside `gtk_imports`). Add `.{ .name = "build_options", .module = build_opts.createModule() }` to those `createModule` `imports` arrays. The conformance target (Task 6) will build with `backend=null`.

- [ ] Create `src/null_backend.zig` — an in-memory backend exposing the **same public surface** `tree.apply` calls: `createWidget`, `connectButtonClick`, `appendChild`, `setText`, `removeChild`, `insertBefore`, `setVisible`, `applyProps`, plus `getWindow`/`setEventSink` for signature parity (no-ops). Records every operation into an inspectable global registry keyed by a synthetic node handle. It imports **no `gtk`** — the "widget" is an opaque handle. Content:

```zig
const std = @import("std");

// The null backend models a widget as an index into `nodes`. `tree.zig` holds
// `*Widget` pointers; we hand out `*Node` cast to the opaque widget pointer the
// tree stores. All state is inspectable for the conformance suite.
pub const Node = struct {
    kind: []const u8,
    text: ?[]const u8 = null,
    visible: bool = true,
    spacing: i64 = 0,
    orientation: []const u8 = "vertical",
    title: ?[]const u8 = null,
    children: std.ArrayList(*Node) = .empty,
    clicked_connected: bool = false,
};

// The tree stores `*Widget`; for the null backend `Widget` == `Node`.
pub const Widget = Node;

var gpa: std.mem.Allocator = undefined;
var initialized = false;
pub var nodes: std.ArrayList(*Node) = .empty;
pub var last_window: ?*Node = null;

pub fn init(allocator: std.mem.Allocator) void {
    gpa = allocator;
    initialized = true;
    nodes = .empty;
    last_window = null;
}

pub fn reset() void {
    for (nodes.items) |n| {
        n.children.deinit(gpa);
        gpa.destroy(n);
    }
    nodes.clearRetainingCapacity();
    last_window = null;
}

fn propStr(props: ?std.json.Value, key: []const u8) ?[]const u8 {
    const v = props orelse return null;
    if (v != .object) return null;
    const field = v.object.get(key) orelse return null;
    return switch (field) {
        .string => field.string,
        else => null,
    };
}
fn propInt(props: ?std.json.Value, key: []const u8) ?i64 {
    const v = props orelse return null;
    if (v != .object) return null;
    const field = v.object.get(key) orelse return null;
    return switch (field) {
        .integer => field.integer,
        else => null,
    };
}

pub fn setEventSink(_: *const fn (node_id: u32) void) void {}
pub fn getWindow() ?*Node {
    return last_window;
}

// Signature-parallel to gtk_backend.createWidget: same (app, kind, props) shape.
// `app` is unused (opaque *anyopaque so callers pass whatever they hold).
pub fn createWidget(_: *anyopaque, kind: []const u8, props: ?std.json.Value) !*Node {
    const node = try gpa.create(Node);
    node.* = .{ .kind = kind };
    if (std.mem.eql(u8, kind, "Window")) {
        node.title = propStr(props, "title");
        last_window = node;
    } else if (std.mem.eql(u8, kind, "Box")) {
        node.orientation = propStr(props, "orientation") orelse "vertical";
        node.spacing = propInt(props, "spacing") orelse 0;
    } else if (std.mem.eql(u8, kind, "Label")) {
        node.text = propStr(props, "text") orelse "";
    } else if (std.mem.eql(u8, kind, "Button")) {
        node.text = propStr(props, "label") orelse "Button";
    } else {
        gpa.destroy(node);
        return error.UnknownWidget;
    }
    try nodes.append(gpa, node);
    return node;
}

pub fn connectButtonClick(button: *Node, _: u32) void {
    button.clicked_connected = true;
}

pub fn appendChild(parent: *Node, child: *Node) void {
    parent.children.append(gpa, child) catch {};
}

pub fn setText(widget: *Node, text: []const u8) void {
    widget.text = text;
}

pub fn removeChild(parent: *Node, child: *Node) void {
    for (parent.children.items, 0..) |c, i| {
        if (c == child) {
            _ = parent.children.orderedRemove(i);
            return;
        }
    }
}

pub fn insertBefore(parent: *Node, child: *Node, before: ?*Node) void {
    if (before) |b| {
        for (parent.children.items, 0..) |c, i| {
            if (c == b) {
                parent.children.insert(gpa, i, child) catch {};
                return;
            }
        }
    }
    parent.children.append(gpa, child) catch {};
}

pub fn setVisible(widget: *Node, visible: bool) void {
    widget.visible = visible;
}

pub fn applyProps(widget: *Node, kind: []const u8, props: ?std.json.Value) void {
    if (std.mem.eql(u8, kind, "Box")) {
        if (propInt(props, "spacing")) |s| widget.spacing = s;
    } else if (std.mem.eql(u8, kind, "Window")) {
        if (propStr(props, "title")) |t| widget.title = t;
    }
}
```

> **Type-compatibility note for the seam.** `tree.zig` today types nodes as `*gtk.Widget`. For the comptime swap to compile against the null backend, `tree.zig` must refer to the backend's widget type, not `gtk.Widget`, in the two places it names it (`nodes: AutoHashMapUnmanaged(u32, *gtk.Widget)` and the `?*gtk.Widget` locals in `apply`). Introduce `const Widget = backend.Widget;` in `tree.zig` and use `*Widget` throughout `apply`/`nodes`; add `pub const Widget = gtk.Widget;` to `gtk_backend.zig` (the null backend already exports `pub const Widget = Node`). This keeps the GTK build identical (`*Widget` == `*gtk.Widget`) and lets the null build substitute `*Node`. **`automation.zig` still names `gtk.Widget` directly and imports `gtk_backend` directly — it is unaffected.** `createWidget`'s first param: `gtk_backend` takes `*gtk.Application`; `null_backend` takes `*anyopaque`. Have `tree.zig`'s null-path not apply — instead, make `gtk_backend.createWidget` accept the app and `tree.zig` pass `self.app.?`; for the null backend, `conformance.zig` calls `createWidget` directly with a dummy pointer (see Task 6), so `tree.apply`'s create arm is exercised by the GTK build and conformance drives the null backend through its own harness. **This means conformance does NOT reuse `tree.apply`; it drives `null_backend` functions directly from a schema-driven loop** — simpler, and it isolates schema-semantics testing from the reconciler. Record this in the plan: the seam exists so `null_backend` presents an identical surface, but conformance calls that surface directly rather than through `tree.apply`.

- [ ] Build the default (gtk) path to confirm the seam is transparent:
```bash
nix develop -c zig build
nix develop -c zig build test
```
Expected: both succeed (the gtk branch is selected by default; behavior unchanged).

**Commit:**
```bash
git add src/backend.zig src/null_backend.zig src/tree.zig build.zig
git commit -m "feat: backend seam + in-memory null backend"
```

### Interfaces (produced by this task)
- `src/backend.zig` — `pub const impl` = comptime-selected backend module.
- `src/null_backend.zig` — same op surface as `gtk_backend`, with inspectable `nodes`, `init`/`reset`.
- `gtk_backend.Widget` = `gtk.Widget`; `null_backend.Widget` = `null_backend.Node`.

---

## TASK 6 — Schema-driven conformance suite (`src/conformance.zig`) + build target

**Track B. Depends on: Task 5. Files: `src/conformance.zig`, `build.zig`.**

- [ ] Create `src/conformance.zig` — it `@embedFile`s `schema/widgets.json`, parses it, and for each widget drives the null backend directly: create-with-defaults, then apply each `createAndUpdate` prop and assert recorded state; for containers, append/insertBefore/remove and assert ordering. Content:

```zig
const std = @import("std");
const nb = @import("null_backend.zig");

const schema_json = @embedFile("../schema/widgets.json");

// A dummy non-null pointer for the createWidget `app` slot (null backend ignores it).
fn dummyApp() *anyopaque {
    return @ptrFromInt(@alignOf(usize));
}

test "schema drives create-with-defaults for every widget" {
    const gpa = std.testing.allocator;
    nb.init(gpa);
    defer nb.reset();

    const parsed = try std.json.parseFromSlice(std.json.Value, gpa, schema_json, .{});
    defer parsed.deinit();
    const widgets = parsed.value.object.get("widgets").?.array;

    for (widgets.items) |w| {
        const name = w.object.get("name").?.string;
        // create with no props -> defaults must hold
        const node = try nb.createWidget(dummyApp(), name, null);
        if (std.mem.eql(u8, name, "Box")) {
            try std.testing.expectEqual(@as(i64, 0), node.spacing);
            try std.testing.expectEqualStrings("vertical", node.orientation);
        } else if (std.mem.eql(u8, name, "Label")) {
            try std.testing.expectEqualStrings("", node.text.?);
        } else if (std.mem.eql(u8, name, "Button")) {
            try std.testing.expectEqualStrings("Button", node.text.?);
        } else if (std.mem.eql(u8, name, "Window")) {
            try std.testing.expect(node.title == null);
        }
    }
}

test "createAndUpdate props apply against the null backend" {
    const gpa = std.testing.allocator;
    nb.init(gpa);
    defer nb.reset();

    const box = try nb.createWidget(dummyApp(), "Box", null);
    var buf: [64]u8 = undefined;
    var fbs = std.io.fixedBufferStream(&buf);
    _ = fbs;
    // apply spacing=12 via a parsed json.Value props object
    const props = try std.json.parseFromSlice(std.json.Value, gpa, "{\"spacing\":12}", .{});
    defer props.deinit();
    nb.applyProps(box, "Box", props.value);
    try std.testing.expectEqual(@as(i64, 12), box.spacing);

    const win = try nb.createWidget(dummyApp(), "Window", null);
    const wprops = try std.json.parseFromSlice(std.json.Value, gpa, "{\"title\":\"Hi\"}", .{});
    defer wprops.deinit();
    nb.applyProps(win, "Window", wprops.value);
    try std.testing.expectEqualStrings("Hi", win.title.?);
}

test "container append/insertBefore/remove ordering" {
    const gpa = std.testing.allocator;
    nb.init(gpa);
    defer nb.reset();

    const box = try nb.createWidget(dummyApp(), "Box", null);
    const a = try nb.createWidget(dummyApp(), "Label", null);
    const b = try nb.createWidget(dummyApp(), "Label", null);
    const c = try nb.createWidget(dummyApp(), "Label", null);

    nb.appendChild(box, a);
    nb.appendChild(box, c);
    nb.insertBefore(box, b, c); // -> [a, b, c]
    try std.testing.expectEqual(@as(usize, 3), box.children.items.len);
    try std.testing.expect(box.children.items[0] == a);
    try std.testing.expect(box.children.items[1] == b);
    try std.testing.expect(box.children.items[2] == c);

    nb.removeChild(box, b); // -> [a, c]
    try std.testing.expectEqual(@as(usize, 2), box.children.items.len);
    try std.testing.expect(box.children.items[0] == a);
    try std.testing.expect(box.children.items[1] == c);
}

test "setText and setVisible record on the null backend" {
    const gpa = std.testing.allocator;
    nb.init(gpa);
    defer nb.reset();
    const lbl = try nb.createWidget(dummyApp(), "Label", null);
    nb.setText(lbl, "hello");
    try std.testing.expectEqualStrings("hello", lbl.text.?);
    nb.setVisible(lbl, false);
    try std.testing.expect(lbl.visible == false);
}
```

> **Note on the unused `fbs` scaffold:** delete the `var buf`/`fbs`/`_ = fbs;` lines — they are a leftover; the props are parsed directly from a JSON literal. (Kept out of the final file.)

- [ ] Add the conformance test target to `build.zig`, built with `backend=null` (so it pulls `null_backend`, no gtk imports):
```zig
const conformance_tests = b.addTest(.{
    .root_module = b.createModule(.{
        .root_source_file = b.path("src/conformance.zig"),
        .target = target,
        .optimize = optimize,
    }),
});
test_step.dependOn(&b.addRunArtifact(conformance_tests).step);
```
> `conformance.zig` imports only `null_backend.zig` + std — **no gtk imports and no build_options** (it names `null_backend` directly, not through the seam), so its module needs no `imports`. This is the minimal test wiring.

- [ ] Run the full test step (includes conformance now):
```bash
nix develop -c zig build test
```
Expected: all tests pass, including the four conformance tests. Confirm they appear by running with summary:
```bash
nix develop -c zig build test --summary all 2>&1 | rg -i "conformance|passed|failed" | head
```

**Commit:**
```bash
git add src/conformance.zig build.zig
git commit -m "test: schema-driven null-backend conformance suite"
```

### Interfaces (produced by this task)
- `src/conformance.zig` — a `zig build test` target pinning schema semantics against `null_backend` with no display server.

---

## TASK 7 — CI codegen-freshness gate + full-gate integration

**Depends on: Tasks 4 and 6 (both tracks green). Files: `.github/workflows/ci.yml`, `CLAUDE-activeContext.md`.**

- [ ] Insert a `codegen freshness` step into `.github/workflows/ci.yml` **before** the `build` step (after `unit tests`). It regenerates and fails on any drift:
```yaml
      - name: codegen freshness
        run: |
          nix develop -c bun tools/codegen.ts
          git diff --exit-code -- schema packages/react/src/generated src/generated docs/widgets.md
```
> Placing it after `unit tests` (which now includes conformance) and before `build` means a stale generated file fails fast. `git diff --exit-code` exits non-zero on any uncommitted change to the generated paths.

- [ ] Update the **full gate** command in `CLAUDE-activeContext.md` to include a codegen freshness check, so the local gate matches CI. Change the M4 gate line to prepend:
```bash
nix develop -c bash -c 'bun tools/codegen.ts && git diff --exit-code -- schema packages/react/src/generated src/generated docs/widgets.md && zig build test && zig build && bun install --frozen-lockfile && ./scripts/headless-smoke.sh && ./scripts/headless-m2.sh && ./scripts/kill9-test.sh && ./scripts/headless-m3.sh && ./scripts/headless-m4.sh'
```
Also add a one-line M5a entry to the State section noting: schema/codegen foundation landed, four widgets generated, null backend + conformance green, `nd codegen` = `bun tools/codegen.ts`.

- [ ] Run the **entire** updated gate locally and confirm green:
```bash
nix develop -c bash -c 'bun tools/codegen.ts && git diff --exit-code -- schema packages/react/src/generated src/generated docs/widgets.md && zig build test && zig build && bun install --frozen-lockfile && ./scripts/headless-smoke.sh && ./scripts/headless-m2.sh && ./scripts/kill9-test.sh && ./scripts/headless-m3.sh && ./scripts/headless-m4.sh'
```
Expected: no diff from codegen, all tests pass, all five headless scripts pass.

- [ ] Confirm `node_modules/` and build caches are not staged:
```bash
git status --porcelain | rg "node_modules|zig-cache|.zig-cache|zig-out" || echo "clean"
```
Expected: `clean`.

**Commit:**
```bash
git add .github/workflows/ci.yml
git commit -m "ci: codegen freshness gate"
# CLAUDE-activeContext.md is intentionally NOT committed (memory-bank policy);
# update it in place but leave it unstaged.
```

### Interfaces (produced by this task)
- CI step `codegen freshness` — re-runs codegen and `git diff --exit-code` on generated paths.
- `nd codegen` documented as `bun tools/codegen.ts`.

---

## Self-review checklist (run before declaring M5a done)

- [ ] **D6 satisfied:** the four widget appliers exist ONLY as codegen output. `rg -n "gtk.ApplicationWindow.new|gtk.Box.new|gtk.Label.new|gtk.Button.newWithLabel" src/` returns hits **only** in `src/generated/widgets.zig` (and `src/main.zig`'s unrelated `--smoke` path, which is the M1 pure-GTK demo — leave it). No per-widget create/apply logic remains hand-written in `gtk_backend.zig`.
- [ ] **Determinism:** `bun tools/codegen.ts` twice → `git diff --exit-code` clean. Prop/widget order in every generated file matches `schema/widgets.json` declaration order.
- [ ] **Generated headers:** every generated file's first line is the `// GENERATED …` (or `<!-- GENERATED … -->` for markdown) marker.
- [ ] **Byte-identical runtime:** `headless-m3.sh` + `headless-m4.sh` pass unchanged — the counter renders and drives identically. If any GTK call order changed, the generated body diverged from M4; fix in `codegen.ts`.
- [ ] **No new npm deps:** `tools/codegen.ts` imports only `node:path` + Bun globals; `bun.lock` is unchanged by M5a (no new dependency lines).
- [ ] **Backend seam is comptime, not a vtable:** `src/backend.zig` is a comptime `if`; the shipping exe selects `gtk` with zero indirection; conformance selects `null` (or calls it directly). No C-ABI vtable was introduced (that is M6+ per spec §7).
- [ ] **Conformance is display-server-free:** `zig build test` alone runs the four conformance tests; no weston, no `NATIVE_AUTOMATION`.
- [ ] **`automation.zig` untouched:** it still `@import("gtk_backend.zig")` and names `gtk.Widget`; it never builds against null. `rg -n "backend.zig" src/automation.zig` → no hits.
- [ ] **Public Zig surface unchanged:** `tree.zig`/`automation.zig` compile without signature changes to the backend functions they call.
- [ ] **Explicit-path commits only:** no `git add -A`; `node_modules`/caches never staged; `CLAUDE.md`/`CLAUDE-*.md` never committed.
- [ ] **Schema anticipates M5b without over-reaching:** enum (`orientation`), container childModel (single/multi), and an events slot are all expressed in v1, but only the four implemented widgets are present. The codegen `throw`s a clear error if a widget/prop template is missing (so M5b fails loudly, not silently, when adding widgets without templates).
