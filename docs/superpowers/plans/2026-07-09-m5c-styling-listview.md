# M5c — Styling + ListView + Codegen Polish: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Parallelism note (read before dispatching waves).** M5c is a short spine then a fan-out:
> - **Spine (strictly sequential): Task 1 → Task 2 → Task 3 → Task 4.** Task 1 is the codegen `key`/IntrinsicAttributes fix (touches `tools/codegen.ts` + regenerates). Task 2 lands the whole `style` schema + codegen surface (StyleProp TS type, style-compiler table, Zig style module). Task 3 wires the Zig style compiler into the host apply path (`src/style.zig` NEW + `src/gtk_backend.zig` + `src/tree.zig`). Task 4 lands the ListView widget end-to-end (schema + codegen + host wiring). Tasks 2–4 all regenerate `src/generated/widgets.zig`, so they MUST NOT run concurrently.
> - **Fan-out after Task 4 (disjoint files, safe in parallel):**
>   - **Task 5 — TS validator track:** `packages/react/src/style-validate.ts` (NEW) + `host-config.ts` wiring + a bun unit test. TS-only.
>   - **Task 6 — conformance track:** `src/conformance.zig` (style round-trip + ListView itemCount assertions) + any `src/null_backend.zig` support. Zig-only.
>   - **Task 7 — gallery track:** `examples/gallery/*` only (styled section + ListView tab + revert the `createElement` workaround to plain JSX).
>   - **Task 8 — docs track:** the generated `docs/styling.md` emitter lives in `tools/codegen.ts` (so it belongs to Task 2); this task only writes the hand-authored prose intro if any. Fold into Task 2 unless a gap appears.
> - **Task 9 (drive script + headless script + CI) depends on Tasks 5, 6, 7. Task 10 (full gate + docs/memory sync + self-review) runs last.**
>
> This plan completes spec §14 **M5** (the styling + ListView remainder after M5a schema/codegen and M5b's 18 widgets). It adds **no new milestone scope** — macOS styling/AppKit, Menu, and React item-templates for ListView are explicitly OUT (the React-template deferral is justified in M5c-D5 below).

**Goal:** Three deliverables, all through the D6 codegen mandate (zero hand-written per-widget bindings):

1. **Styling (spec §6, the marquee agent-facing feature).** A shared, typed, non-web `style` prop on **every** widget — colors, font, spacing, border — compiled to GTK CSS + widget margin properties, with **web-CSS rejected loudly**: unknown style keys (`flex`, `position`, `display`, `justifyContent`, …) fail at the React renderer (TypeScript type + a runtime validator with a fix-it message naming the nearest valid key) AND defensively host-side (unknown key → `ND_WARN` + structured error event, never silent). Generated reference page `docs/styling.md`.
2. **ListView (spec §6, the M5 100k-row recycling demo).** A data-driven `ListView` widget (`GtkListView` + `GtkStringList` + `GtkSignalListItemFactory` native-templated string rows), 100k rows mounted from React state via one props update, scrolled by the automation `scroll` action, with `selectedIndex` + `onRowActivated` wired. `getTree` reports `itemCount`, **not** 100k children.
3. **Codegen polish.** Emit `key?: React.Key` on all intrinsics by declaring `JSX.IntrinsicAttributes` in the generated namespace (the M5b gap), and revert the gallery's `createElement` workaround to plain JSX.

**Architecture:** Unchanged two-process topology (D1). Two generalizations:
- **Styling is a cross-cutting prop, not a widget.** The `style` object rides the child's create/update-op props (no protocol change). The host compiles it to a GTK CSS block scoped to a per-node class `nd-<id>` and installs it via one `GtkCssProvider` per styled node at display level (see **M5c-D1** for the provider strategy and **M5c-D2** for the margin/CSS split). The `style` prop is `appliesTo: "createAndUpdate"` and handled by a dedicated `src/style.zig` compiler called from `gtk_backend.applyStyle`, invoked from `tree.apply` at both create and update.
- **ListView is a self-contained scroller.** Its create body wraps a `GtkListView` (with a `GtkSignalListItemFactory` producing `GtkLabel` rows over a `GtkStringList` model in a `GtkNoSelection`/`GtkSingleSelection`) inside a `GtkScrolledWindow` and returns the ScrolledWindow (so the existing `scroll` automation action drives it for free — **M5c-D3**). `container: null` (data-driven; children are the `items` string array, not JSX children). `itemCount` is stashed in `NodeMeta` for `getTree` (**M5c-D4**).

**Tech Stack:** unchanged — Zig 0.16.0 exact, vendored zig-gobject at `vendor/gobject-bindings`, Bun 1.3.13, GTK4 4.22.4 (devshell), TypeScript strict, weston headless + `GSK_RENDERER=cairo` for display-dependent scripts, plain `zig build test` for conformance. `tools/codegen.ts` keeps zero npm deps.

## Global Constraints

Carried over from M1–M5b, unchanged:

- Zig exactly `0.16.0` (`build.zig` `checkZigVersion()` stays). Bun pinned 1.3.13.
- No `@cImport`; all GTK via vendored `glib`/`gobject`/`gio`/`gtk`/`gdk`/`gsk` modules.
- **No hand-written per-widget bindings (D6).** Every per-widget create/apply/signal-wire/attach body exists ONLY in `src/generated/widgets.zig`, emitted by `tools/codegen.ts` from `schema/widgets.json`. The style compiler is cross-cutting (not per-widget) and lives in a hand-written `src/style.zig` — this is allowed: it is not a widget binding, it is a prop-compiler shared by all widgets, exactly as `src/automation.zig`'s semantic actions are shared runtime host code. Editing `src/generated/*` by hand is a policy violation.
- Generated files carry the `// GENERATED by tools/codegen.ts — do not edit` first line; deterministic, byte-stable, committed; CI freshness gate (`bun tools/codegen.ts && git diff --exit-code -- schema packages/react/src/generated src/generated docs/widgets.md docs/styling.md`) — **note the added `docs/styling.md` path** — stays and must stay green after every task that regenerates.
- Markers (`ND_*`) print to **stderr**; scripts capture `2>&1`. Each headless script uses a unique weston socket name (`nd-headless-m5c`).
- Commit style: short imperative lowercase subject, no co-author trailers. `git add` **explicit paths per task** — never `git add -A`; `node_modules`/caches/`CLAUDE*.md` never staged.
- All commands inside the devshell (`nix develop -c` in CI; direnv locally).
- **Full gate** (run before starting and after Task 10; Task 10 extends it with `headless-m5c.sh` and the added `docs/styling.md` freshness path):
```bash
nix develop -c bash -c 'bun tools/codegen.ts && git diff --exit-code -- schema packages/react/src/generated src/generated docs/widgets.md docs/styling.md && zig build test && zig build && bun install --frozen-lockfile && ./scripts/headless-smoke.sh && ./scripts/headless-m2.sh && ./scripts/kill9-test.sh && ./scripts/headless-m3.sh && ./scripts/headless-m4.sh && ./scripts/headless-m5b.sh'
```
- Counter app and every existing gate stay green after **every** task. **Regression contract: a widget with no `style` prop is byte-identical to M5b** — the style compiler is only invoked when `props.style` is present; absent style = zero CSS provider, zero margin call, zero behavior change. The M3/M4/M5b headless scripts are the standing regression proof.

### Zig 0.16 drift facts that bite this milestone (from CLAUDE-activeContext.md, all verified in-repo)

- `std.ArrayList(T)` is unmanaged: `var xs: std.ArrayList(T) = .empty;` + `xs.append(alloc, item)`. The style-CSS string is built with `std.ArrayList(u8)` + `writer`, or `std.fmt.allocPrint` piecewise into an arena.
- No wall-clock in tests: `std.Io.sleep(io, .fromMilliseconds(n), .awake)` (already at `src/automation.zig:686`).
- `std.mem.Allocator.dupeZ` exists — the style compiler needs NUL-terminated CSS text for `loadFromString` and NUL-terminated class names for `addCssClass`.
- `std.json.Value` object access is `v.object.get(key)`; nested style objects are read the same way the existing `propObj`/`paramObj` helpers do (`src/automation.zig:34`).
- The generated Zig imports `gdk` and needs it for `gtk.Widget.getDisplay` → `gdk.Display`; `src/style.zig` imports `gtk`/`gdk`/`gobject`/`glib` the same way `gtk_backend.zig` does.

### M5c decision record (owner-facing judgment calls, locked by this plan)

- **M5c-D1 — Provider strategy: one `GtkCssProvider` per styled node, unique class `nd-<id>`, installed once at display level; update = replace provider content.** Verified bindings (symbol table below): `gtk.CssProvider.new()`, `gtk.CssProvider.loadFromString(provider, [*:0]const u8)` (GTK 4.22 — no `GError**`, parse errors surface via the `parsing-error` signal, gtk4.zig:11937), `gtk.StyleContext.addProviderForDisplay(*gdk.Display, *gtk.StyleProvider, c_uint)` (gtk4.zig:46220), `gtk.Widget.addCssClass(*Widget, [*:0]const u8)` (gtk4.zig:56860), `gtk.Widget.getDisplay(*Widget) *gdk.Display` (gtk4.zig:57189), priority const `STYLE_PROVIDER_PRIORITY_APPLICATION = 600` (gtk4.zig:76694). **Chosen because:** (a) the GTK-4.16+ widget-level `gtk_widget_add_provider`/style-context-per-widget API is NOT in these bindings (only the display-level `addProviderForDisplay` is) — so display-level is the only verified route; (b) a unique `nd-<id>` class selector keeps each node's rules isolated even though the provider is display-wide (the CSS body is `.nd-<id> { … }`, so it only matches the one widget carrying that class); (c) on update we hold the node's `*GtkCssProvider` in a host-side `id → provider` map and call `loadFromString` again with the new CSS body — replacing the provider's rules in place, no add/remove churn; the provider was already added to the display at first-style time. **Provider lifetime:** the provider is created lazily on the first styled `applyStyle` for a node, added to the display once, stored in the map, and reused for every later update. A node that never carries `style` gets no provider. Providers are process-lifetime (arena-owned map, acceptable v1 leak, documented) — nodes are removed rarely and the CSS body for a removed node's class simply stops matching anything.
- **M5c-D2 — Margin maps to widget properties; everything else maps to CSS.** GTK widget margins are *widget properties* (`gtk_widget_set_margin_start/end/top/bottom`, gtk4.zig:58187/58183/58191/58179), **NOT** CSS (`margin` in a GtkCssProvider block on a widget is unreliable/ignored for top-level widget spacing — research gotcha, and GTK's own docs treat widget margins as properties). So the style compiler splits the object: **`margin` → `set_margin_*` calls on the widget; `padding`, `background`, `color`, `font*`, `border*` → CSS rules in the `.nd-<id>` block.** Both halves are driven from the same `style` object in one `applyStyle` pass. `padding` legitimately IS a GTK CSS property, so it stays in CSS. This split is documented in `docs/styling.md` and asserted by conformance.
- **M5c-D3 — ListView is a self-contained scroller (GtkListView inside a GtkScrolledWindow), create returns the ScrolledWindow.** `GtkListView` implements `gtk.Scrollable` (gtk4.zig:29968) — it does not scroll itself; it needs a `GtkScrolledWindow` viewport. The create body builds the inner `GtkListView` + factory + model + selection, wraps it with `gtk.ScrolledWindow.setChild`, and returns the ScrolledWindow as the tracked widget. **Payoff:** the existing `scroll` automation action already targets `ScrolledWindow` (`src/automation.zig:298` `handleScroll`) — so scrolling a 100k-row ListView is free, no new automation action. The inner `*GtkListView` is recovered in `applyProps`/`connectEvents` via `gtk.ScrolledWindow.getChild` (gtk4.zig:40336). The `minContentHeight` styling of the scroller is left at GTK defaults (the demo sets a window size); a future prop can expose it.
- **M5c-D4 — `getTree` reports ListView `itemCount`, never its rows.** GtkListView recycles ~200 live row widgets for millions of items (research: "roughly 200 live widgets even for millions of items") — the recycled rows are GTK-internal, untracked, and transient; dumping them would be both wrong (they're not app nodes) and a denial-of-service on the agent (100k entries). `NodeMeta` gains an optional `item_count: ?u32` field, set from the `items.len` (or `itemCount` prop) at create/update. `handleGetTree`'s `collectTrackedChildren` already skips untracked GTK children (M5b), so the recycled rows never appear. `buildNode` emits `itemCount` on the JSON node when the meta has it (null otherwise). The ListView node therefore appears as a single leaf with `type: "ListView"`, `itemCount: 100000`, and `children: []`.
- **M5c-D5 — React item-templates for ListView are DEFERRED to a named follow-up (M5c-D-followup); v1 ships native-templated string rows.** Spec §6 describes "a React item-template component driving setup/bind/unbind". **Honest analysis of why this cannot ship in v1:** GtkSignalListItemFactory's `setup`/`bind`/`unbind` callbacks fire **synchronously on the GTK main thread** at the exact moment a row scrolls into view — the factory blocks the frame until `bind` returns with a populated widget. Driving `bind` through React means: (1) marshal a "bind row N" request across the NDP socket to the Bun child, (2) let React render the item-template component for that row, (3) marshal the resulting widget subtree back as CommitBatch ops, (4) apply them on the UI thread — an inherently **asynchronous** round-trip that cannot satisfy the factory's **synchronous** bind-time contract. The row would be blank until the async React commit lands a frame or more later, defeating recycling's whole purpose. Making it work needs the spec-D10 geometry/sync design (a synchronous sub-protocol or an in-process N-API path) plus a pre-rendered-template + data-only-bind model where React renders the *template* once and the host binds *data* into a fixed widget shape synchronously. That is a design task, not an implementation task. **v1 = native GtkLabel rows bound from a flat `string[]`**, which fully exercises the recycling machine, the 100k-row demo, scrolling, and `rowActivated` — everything the M5 milestone actually demos. The follow-up is named **M5c-D-followup: React item-templates for ListView** and referenced in `docs/styling.md`/`docs/widgets.md` and CLAUDE-activeContext.
- **M5c-D6 — `onRowActivated` via the ListView `activate` signal; `selectedIndex` via the selection model.** `GtkListView` has an `activate` signal (name `"activate"`, gtk4.zig ListView.signals.activate, emits the activated **position**). The row-activation payload is the row index → a new event payload kind `"index"` (already exists — reused from Select's `selectionChanged`). `selectedIndex` (createAndUpdate) drives a `GtkSingleSelection` (gtk4.zig:43221, `setSelected`/`getSelected` at 43282/43241); when no `selectedIndex`/`onRowActivated` is present the model is a `GtkNoSelection` (gtk4.zig:32475) — but to keep one create path, v1 **always** uses `GtkSingleSelection` (it also implements `gtk.SelectionModel`, gtk4.zig:43163) so `selectedIndex` works uniformly; unselected default is `GTK_INVALID_LIST_POSITION` unless `selectedIndex` given. The `activate` callback emits `rowActivated {index}` up to React.
- **M5c-D7 — Style validation is dual-layer, TS is primary.** **TS layer (primary, the fix-it feature):** a runtime `validateStyle(style)` in `packages/react/src/style-validate.ts` runs in `commitUpdate`/create emission (host-config) whenever a `style` prop is present. Unknown top-level keys (anything not in `background|color|font|padding|margin|border`) or unknown nested keys throw a `StyleError` with a fix-it message: `Invalid style key "display" — GTK styling is not web CSS. Did you mean one of: background, color, font, padding, margin, border? See docs/styling.md` (nearest-key suggestion by Levenshtein against the valid set; when nothing is close, list all valid keys). The TypeScript `StyleProp` type on every intrinsic makes most hallucinations a compile error; the runtime validator catches dynamically-built style objects. **Host layer (defensive):** `src/style.zig`'s compiler treats any unknown key as `ND_WARN unknown style key "<k>" (GTK is not web CSS)` and emits a structured error Event (`name: "styleError"`, payload carries the offending key + node id) — never a silent drop, never a crash. The host layer exists so a non-React or malicious runtime can't smuggle bad CSS past the type system.

### Landed-code reality (authoritative — verified this session)

| Fact | Landed reality (file:line) |
|---|---|
| Codegen artifacts + throw-sites to extend | `tools/codegen.ts`: `tsTypeOf` :39, `tsHandlerType` :50, `collectAttachedProps` :61, `genIntrinsics` :79 (JSX namespace :85-99), `genSchemaMeta` :104, `genZig` :222, `genZigCreateBody` :280 (throws :392), `genZigApplyBody` :397 (throws :480), `SIGNALS` table :487, `CALLBACK_BODIES` :498, `genZigEvents` :555 (throws :560), `STRUCTURAL` :602, `genZigStructural` :651 (throws :654), `genDocs` :707, writes :751-755. Prop types today: `string|int|bool|enum|float|stringList`. |
| Generated TS JSX namespace (NO IntrinsicAttributes) | `packages/react/src/generated/intrinsics.ts:9-34` — `export namespace JSX { export interface IntrinsicElements {…} export type Element = ReactNode; export interface ElementChildrenAttribute {…} }`. **No `IntrinsicAttributes` interface** → `key` untyped on intrinsics (the M5b gap). `jsx-runtime.ts:6` re-exports `{ jsx, jsxs, Fragment, JSX }`. |
| Generated Zig surface | `src/generated/widgets.zig`: `create(app, kind, props, dupeZ, the_window)`, `applyProps(widget, kind, props, dupeZ)`, `connectEvents(widget, kind, node_id)`, `appendChild`/`insertBefore`/`removeChild`, `initEvents(gpa, emit_fn)`; imported at `src/gtk_backend.zig:5`, delegated at :20/:35/:59/:38/:47/:51. |
| gtk_backend seam | `src/gtk_backend.zig`: `createWidget` :34, `applyProps` :59, `connectEvents` :22, `setText` :42, `getWindow` :26, arena `dupeZ` :30. Style has NO seam yet — add `applyStyle(widget, id, style_value)` here (Task 3). |
| Tree apply path | `src/tree.zig:124` `apply`: `create` :127 (calls createWidget, connectEvents, putMeta), `update` :149 (calls applyProps, refreshes meta text/testID). `NodeMeta` :12 `{widget_type, test_id, text, parent, attached}`. Style is invoked from BOTH create (:137 area) and update (:151 area) — see Task 3. |
| Protocol Op / EventPayload | `src/protocol.zig`: `EventPayload {text,checked,value,index}` :30, `Event` :37, `Op {op,id,widget,props,parent,child,text,before}` :73. `style` rides `props` (a `std.json.Value` object) — no Op change. A `styleError` event reuses `Event.name` (free string) + `EventPayload` (needs a `key: ?[]const u8` field — Task 2 adds it). |
| Runtime event sink | `src/runtime.zig:88` `sendEvent(node_id, name, payload)`; `EmitFn` in generated state. The host style-error event uses the SAME sink (Task 3 passes `emit` into style.zig, or style.zig calls back via a function pointer set at init — mirror `initEvents`). |
| Automation getTree / scroll | `src/automation.zig`: `JsonNode {ref,type,testID,text,visible,geometry,children}` :394 — **add `itemCount: ?u32`** (Task 4). `buildNode` :447, `collectTrackedChildren` :495 (skips untracked GTK children — already handles recycled rows). `handleScroll` :298 targets `ScrolledWindow` (ListView reuses it, M5c-D3). |
| host-config style handling | `packages/react/src/host-config.ts`: `emitCreateIfNew` :51 (spreads `inst.props`, deletes children/handlers), `commitUpdate` :126 (generic prop diff at :135-139). `style` is a plain prop today — flows through unchanged; Task 5 inserts `validateStyle` at both emission points. `collectHandlers` :34 uses `widgetEvents`. |
| ndp.ts Op type | `runtime/ndp.ts:6-14` `Op` union — `create.widget` is `"Window"|"Box"|"Label"|"Button"` (a stale literal union; `props` is `Record<string,unknown>` so `style` already fits). Widen the `widget` literal or leave it (host-config casts). `EventMsg.payload` :16 is `object` — `styleError`/`rowActivated` payloads pass through opaquely. |
| Gallery createElement workaround | `examples/gallery/main.tsx:2` imports `createElement`; :61 `Array.from(... createElement("label", { key: i, text: … }))` — the M5b workaround for untyped `key`. Task 7 reverts to `<label key={i} text={…} />` once Task 1 lands IntrinsicAttributes. |
| Conformance schema-driven | `src/conformance.zig` reads `@import("build_options").schema_json`; `synthValue` :14 (per prop type), generic create/update loops. `null_backend.zig` records props generically as canonical JSON strings. Task 6 adds a `style` synth rule + a ListView `itemCount` assertion path. |
| Scripts/CI | `scripts/headless-m5b.sh` (weston `nd-headless-m5b`, socket-from-log, `m5b-drive.ts`, `M5B_DRIVE_OK`, PNG check) + `scripts/m5b-drive.ts` (`AutomationClient` from `packages/mcp/src/socket.ts`, `find` by testID, `waitFor`). CI `.github/workflows/ci.yml` ends at `headless m5b` (:34); append `headless m5c`. |

### Verified-symbol table (all `rg` runs this session against `vendor/gobject-bindings`; re-run inside the devshell if line numbers drift — the symbol name is the contract)

GTK4 (`G = vendor/gobject-bindings/src/gtk4/gtk4.zig`):

| Need | Symbol (Zig binding) | Verify → line |
|---|---|---|
| **Styling** | `gtk.CssProvider.new() *gtk.CssProvider` | `rg -n "gtk_css_provider_new\b" $G` → 11900 |
| | `gtk.CssProvider.loadFromString(*CssProvider, [*:0]const u8)` (GTK 4.22, no GError) | `rg -n "gtk_css_provider_load_from_string\b" $G` → 11937 |
| | `gtk.CssProvider.loadFromData(*CssProvider, [*:0]const u8, isize)` (fallback for pre-4.22) | 11912 |
| | `gtk.CssProvider.signals.parsing_error` (name `"parsing-error"`) — optional loud parse logging | 11884-11885 |
| | `gtk.StyleContext.addProviderForDisplay(*gdk.Display, *gtk.StyleProvider, c_uint)` | `rg -n "gtk_style_context_add_provider_for_display\b" $G` → 46220 |
| | `gtk.Widget.addCssClass(*Widget, [*:0]const u8)` / `removeCssClass` | 56860 / 57961 |
| | `gtk.Widget.getDisplay(*Widget) *gdk.Display` | 57189 |
| | `gtk.Widget.setMarginStart/End/Top/Bottom(*Widget, c_int)` | 58187 / 58183 / 58191 / 58179 |
| | `STYLE_PROVIDER_PRIORITY_APPLICATION` = 600 (`_USER` = 800) | 76694 / 76717 |
| | `CssProvider` implements `StyleProvider` (so `provider.as(gtk.StyleProvider)` compiles) | CssProvider opaque near 11785; `Implements` includes `gtk.StyleProvider` |
| **ListView** | `gtk.ListView.new(?*gtk.SelectionModel, ?*gtk.ListItemFactory) *gtk.ListView` | `rg -n "gtk_list_view_new\b" $G` → 30060 |
| | `gtk.ListView.setModel(*ListView, ?*SelectionModel)` / `setFactory` / `getModel() ?*SelectionModel` | 30120 / 30106 / 30076 |
| | `gtk.SingleSelection.getModel() ?*gio.ListModel` (recover inner StringList on update) | 43235 |
| | `gio.ListModel.getNItems(*ListModel) c_uint` (`gio2.zig:33959`) | gio2 33959 |
| | `gtk.ListView` implements `gtk.Scrollable` (drives scroll via ScrolledWindow) | opaque 29966, Implements 29968 |
| | `gtk.ListView.signals.activate` (name `"activate"`, emits position `c_uint`) | signals block 30028+; name `"activate"` confirmed |
| | `gtk.SignalListItemFactory.new() *gtk.SignalListItemFactory` | `rg -n "gtk_signal_list_item_factory_new\b" $G` → 43133 |
| | factory signals `setup`/`bind`/`unbind`/`teardown` (each passes `*gtk.ListItem`) | 43073 / 43050 / 43115 / 43093 |
| | `gtk.ListItem.setChild(*ListItem, ?*Widget)` / `getChild` / `getItem() ?*gobject.Object` / `getPosition() c_uint` | 29379 / 29316 / 29327 / 29333 |
| | `gtk.StringList.new(?[*]const [*:0]const u8) *StringList` / `append` / `splice` / `getString(c_uint)` | 45920 / 45927 / 45964 / 45942 |
| | `gtk.StringList` implements `gio.ListModel` (so `sl.as(gio.ListModel)`) | opaque 45888, Implements 45890 |
| | `gtk.StringObject.getString(*StringObject) [*:0]const u8` (bind: `getItem` → StringObject) | 46025 |
| | `gtk.SingleSelection.new(?*gio.ListModel) *SingleSelection` / `getSelected` / `setSelected` | 43221 / 43241 / 43282 |
| | `gtk.SingleSelection` implements `gtk.SelectionModel` + `gio.ListModel` (so `.as(gtk.SelectionModel)`) | opaque 43161, Implements 43163 |
| | `gtk.NoSelection.new(?*gio.ListModel)` (NOT used in v1 — SingleSelection uniform, M5c-D6) | 32475 |
| | `gtk.ScrolledWindow.new()` / `setChild` / `getChild` / `getVadjustment` (reused for wrapping + scroll) | 40328 / 40415 / 40336 / 40403 |
| | `gtk.Label.new(?[*:0]const u8)` / `setText` (row widgets) | 27215 / 27557 |

GObject/GLib (`GO = …/gobject2/gobject2.zig`):

| Need | Symbol | Verify → line |
|---|---|---|
| Generic signal connect (returns handler id) — factory + ListView.activate | `gobject.signalConnectData(*Object, [*:0]const u8, Callback, ?*anyopaque, ?ClosureNotify, ConnectFlags) c_ulong` | GO:5677 (used already, `tools/codegen.ts:587`) |
| Type-erased callback type | `gobject.Callback = *const fn () callconv(.c) void` (cast with `@ptrCast`) | GO:6429 |

**Implement-time confirmations (embed in the task where used; each has a bounded fallback):**

| Claim | Verify command | Fallback if it fails |
|---|---|---|
| `loadFromString` exists (GTK ≥ 4.12; devshell is 4.22) | compile Task 3's `src/style.zig` (`zig build`) | use `loadFromData(provider, css_z, -1)` (11912) — identical behavior, older signature |
| `provider.as(gtk.StyleProvider)` upcasts | `zig build` on Task 3 | `@as(*gtk.StyleProvider, @ptrCast(provider))` — C-ABI-identical for a direct implementor |
| ListView `activate` emits the row position as the 1st arg | Task 9's ListView drive (`onRowActivated` → label update) | connect via `gobject.signalConnectData(listview, "activate", …)` and read `gtk.ListView`'s selection `getSelected` inside the callback |
| GtkListView renders under `GSK_RENDERER=cairo` headless with 100k rows | Task 9 headless drive (getTree `itemCount:100000` + screenshot non-empty) | reduce demo to 10k if cairo chokes; note it and file the follow-up (recycling still proven) |
| `bind` callback `getItem` returns a `StringObject` for a StringList model | Task 4 compiles + Task 9 drive shows row text | cast `list_item.getItem()` to `*gtk.StringObject` unconditionally (StringList's item type is always StringObject) |

---

## Schema format extensions (the contract — read before Task 2)

`schema/widgets.json` keeps `"schemaVersion": 1` — all changes are **additive**. Format deltas:

- A new top-level schema key `"style"`: the machine-readable definition of the shared style prop (the valid key set + nested shapes), consumed by codegen to emit the TS `StyleProp` type, the Zig style-key table, and `docs/styling.md`. This keeps the style contract single-sourced like widgets are.
- Every widget implicitly gains a `style?: StyleProp` intrinsic field — codegen injects it into every `IntrinsicElements` entry (like the attached-prop union), NOT a per-widget schema edit.
- A new widget object `ListView` with `"container": null` and a `"dataProps"` marker is unnecessary — `ListView` is a plain widget whose `items` prop is a `stringList`. No new prop type needed.

The `"style"` schema block (Task 2 adds it verbatim after `"schemaVersion"`):

```json
  "style": {
    "keys": {
      "background": { "kind": "color", "css": "background-color" },
      "color":      { "kind": "color", "css": "color" },
      "font": {
        "kind": "object",
        "fields": {
          "fontSize":   { "kind": "int",  "css": "font-size", "unit": "px" },
          "fontWeight": { "kind": "enum", "css": "font-weight", "values": ["normal", "bold"] },
          "fontFamily": { "kind": "string", "css": "font-family" }
        }
      },
      "padding": { "kind": "spacing", "css": "padding", "unit": "px", "target": "css" },
      "margin":  { "kind": "spacing", "target": "widget" },
      "border": {
        "kind": "object",
        "fields": {
          "borderWidth":  { "kind": "int",   "css": "border-width",  "unit": "px" },
          "borderColor":  { "kind": "color", "css": "border-color" },
          "borderRadius": { "kind": "int",   "css": "border-radius", "unit": "px" }
        }
      }
    }
  }
```

`spacing` values are `int` (all sides) or `{ top, right, bottom, left }` (per-side object). `color` values are hex or `rgb()/rgba()` strings passed through to CSS verbatim (GTK CSS accepts both); the compiler does not parse them beyond a cheap sanity check (`starts with '#'` or `starts with "rgb"`). `border` needs `border-style: solid` implied whenever any border field is present (GTK, like CSS, renders no border without a style).

---

## TASK 1 — Codegen polish: `JSX.IntrinsicAttributes` (`key` support)

**Spine. Depends on: nothing. Files: `tools/codegen.ts`, regenerated `packages/react/src/generated/intrinsics.ts`.**

Fixes the M5b gap: intrinsics don't declare `JSX.IntrinsicAttributes`, so `key` (and `ref`) aren't typed on `<label>` etc., forcing the gallery's `createElement` workaround. This is a pure-additive codegen change; runtime behavior is unchanged (React always accepted `key`; only the types were missing).

- [ ] In `tools/codegen.ts`, in `genIntrinsics` (`:79`), inside the emitted `export namespace JSX { … }` block, add an `IntrinsicAttributes` interface after the `IntrinsicElements` interface closes. Locate the line that emits `out += "  }\n";` closing `IntrinsicElements` (`:96`) and the `ElementChildrenAttribute` emission (`:98`); insert between the `Element` type and `ElementChildrenAttribute`:

```ts
  out += "  export type Element = ReactNode;\n";
  out += "  export interface IntrinsicAttributes {\n    key?: string | number | null;\n  }\n";
  out += "  export interface ElementChildrenAttribute {\n    children: {};\n  }\n";
```

(Replace the existing two lines emitting `Element` + `ElementChildrenAttribute` with these three.) `React.Key` is `string | number` (React 19); `null` is allowed by React's own JSX types. Using the literal avoids importing `React.Key` into the generated file (which imports only `ReactNode` from `react`).

- [ ] Regenerate and confirm the diff is ONLY the new interface:

```bash
nix develop -c bun tools/codegen.ts
git diff -- packages/react/src/generated/intrinsics.ts
```
Expected: the diff adds exactly the `IntrinsicAttributes` interface inside `namespace JSX`; nothing else moves.

- [ ] Prove `key` now type-checks on an intrinsic. Add a throwaway check (do NOT commit it): create `examples/gallery/_keycheck.tsx` with `const x = <label key={1} text="a" />;` and run the gallery typecheck:

```bash
nix develop -c bash -c 'cd examples/gallery && bun x tsc --noEmit -p tsconfig.json'
```
Expected: no error on the `key`. Then `rm examples/gallery/_keycheck.tsx`. (The gallery's real `createElement` revert happens in Task 7, after styling is in.)

**Commit:**
```bash
git add tools/codegen.ts packages/react/src/generated/intrinsics.ts
git commit -m "feat(codegen): declare JSX.IntrinsicAttributes so key types on intrinsics"
```

### Interfaces (produced by this task)
- `JSX.IntrinsicAttributes { key? }` in generated intrinsics — unblocks plain-JSX `key` (Task 7 reverts the gallery workaround).

---

## TASK 2 — Style schema + full codegen surface (`StyleProp` TS, Zig style table, `docs/styling.md`)

**Spine. Depends on: Task 1. Files: `schema/widgets.json`, `tools/codegen.ts`, `src/protocol.zig`, regenerated `packages/react/src/generated/{intrinsics,schema-meta}.ts`, `src/generated/widgets.zig`, `docs/styling.md`, `docs/widgets.md`.**

Lands the **entire style contract** as data + codegen output. The host doesn't compile CSS yet (Task 3 wires `src/style.zig`), but the generated TS `StyleProp` type, the `style?` intrinsic field, the Zig style-key table, and `docs/styling.md` all land here and must compile/regenerate cleanly. Runtime behavior after this task is **byte-identical to M5b** — nothing reads `props.style` yet.

### 2a — `schema/widgets.json`: add the `"style"` block

- [ ] Insert the `"style": { … }` block (verbatim from "Schema format extensions" above) immediately after `"schemaVersion": 1,` and before `"widgets"`. Keep every existing widget byte-identical.

### 2b — `src/protocol.zig`: `EventPayload.key` for style errors

- [ ] Add a `key: ?[]const u8 = null` field to `EventPayload` (`:30`) so a `styleError` event can carry the offending style key. It serializes only when set (`emit_null_optional_fields=false` is already used for events), so existing event frames are byte-identical:

```zig
pub const EventPayload = struct {
    text: ?[]const u8 = null,
    checked: ?bool = null,
    value: ?f64 = null,
    index: ?i64 = null,
    key: ?[]const u8 = null,
};
```

- [ ] Add a golden test next to the existing payload tests confirming a key-only payload serializes as `"payload":{"key":"display"}` and the existing `clicked`/`changed` frames are unchanged (mirror `src/protocol.zig:171-188`).

### 2c — `tools/codegen.ts`: types, `StyleProp`, intrinsic injection, Zig table, docs

- [ ] **Schema types.** Extend the `Schema` interface (`:37`) with the optional style block and add style types:

```ts
type StyleKeyKind = "color" | "int" | "enum" | "string" | "object" | "spacing";
interface StyleField { kind: StyleKeyKind; css?: string; unit?: string; values?: string[] }
interface StyleKey extends StyleField { fields?: Record<string, StyleField>; target?: "css" | "widget" }
interface StyleDef { keys: Record<string, StyleKey> }
interface Schema { schemaVersion: number; style?: StyleDef; widgets: Widget[] }
```

- [ ] **`genStyleProp` — the TS type.** Add a new emitter that turns the style block into a `StyleProp` type + a runtime valid-key manifest. It writes into `intrinsics.ts` (so the type is co-located with the JSX namespace):

```ts
function tsStyleField(f: StyleField): string {
  switch (f.kind) {
    case "color": return "string";
    case "string": return "string";
    case "int": return "number";
    case "enum": return (f.values ?? []).map((v) => JSON.stringify(v)).join(" | ");
    case "spacing": return "number | { top?: number; right?: number; bottom?: number; left?: number }";
    case "object": return "{ " + Object.entries(f.fields ?? {}).map(([k, v]) => `${k}?: ${tsStyleField(v)}`).join("; ") + " }";
  }
}

function genStyleProp(s: Schema): string {
  const style = s.style;
  if (!style) return "export type StyleProp = Record<string, never>;\n";
  let out = "export interface StyleProp {\n";
  for (const [k, def] of Object.entries(style.keys)) out += `  ${k}?: ${tsStyleField(def)};\n`;
  out += "}\n";
  // Runtime manifest the validator consumes (nested keys included).
  out += "export const styleKeySpec: Record<string, string[] | null> = {\n";
  for (const [k, def] of Object.entries(style.keys)) {
    const nested = def.kind === "object" ? Object.keys(def.fields ?? {}) : null;
    out += `  ${JSON.stringify(k)}: ${nested ? JSON.stringify(nested) : "null"},\n`;
  }
  out += "};\n";
  return out;
}
```

- [ ] **Inject `style?: StyleProp` into every intrinsic.** In `genIntrinsics` (`:88` field loop), after the attached-prop push, add:

```ts
    fields.push("style?: StyleProp");
```
and, at the top of `genIntrinsics`, emit the `StyleProp` block before the namespace so the type is in scope:

```ts
  out += genStyleProp(s);
  out += "\n";
```
(place it after the `WidgetType` export line, before `export namespace JSX {`).

- [ ] **`genZig` — the style-key table.** After the create/apply dispatchers, emit a data table `src/style.zig` reads via `@import("generated/widgets.zig")`. Add a `genZigStyleTable(s)` emitter and call it in `genZig` (after `genZigStructural`):

```ts
function genZigStyleTable(s: Schema): string {
  const style = s.style;
  if (!style) return "pub const style_keys = [_]StyleKeyDef{};\n";
  let out = "\npub const StyleTarget = enum { css, widget };\n";
  out += "pub const StyleKeyDef = struct { name: []const u8, css: ?[]const u8, target: StyleTarget, kind: []const u8, unit: ?[]const u8 };\n";
  out += "pub const style_keys = [_]StyleKeyDef{\n";
  for (const [k, def] of Object.entries(style.keys)) {
    const target = def.target === "widget" ? ".widget" : ".css";
    const css = def.css ? JSON.stringify(def.css) : "null";
    const unit = def.unit ? JSON.stringify(def.unit) : "null";
    out += `    .{ .name = ${JSON.stringify(k)}, .css = ${css}, .target = ${target}, .kind = ${JSON.stringify(def.kind)}, .unit = ${unit} },\n`;
  }
  out += "};\n";
  // Nested object/border/font sub-fields, flattened for the compiler.
  out += "pub const StyleSubDef = struct { parent: []const u8, name: []const u8, css: []const u8, kind: []const u8, unit: ?[]const u8 };\n";
  out += "pub const style_subkeys = [_]StyleSubDef{\n";
  for (const [k, def] of Object.entries(style.keys)) {
    if (def.kind !== "object") continue;
    for (const [sk, sf] of Object.entries(def.fields ?? {})) {
      const unit = sf.unit ? JSON.stringify(sf.unit) : "null";
      out += `    .{ .parent = ${JSON.stringify(k)}, .name = ${JSON.stringify(sk)}, .css = ${JSON.stringify(sf.css ?? sk)}, .kind = ${JSON.stringify(sf.kind)}, .unit = ${unit} },\n`;
    }
  }
  out += "};\n";
  return out;
}
```

The generated table is DATA only — `src/style.zig` (Task 3) walks it to compile any style object with zero per-key Zig code, honoring D6's spirit for the style surface.

- [ ] **`genStyleDocs` — `docs/styling.md`.** Add an emitter and a new write target. Match `genDocs`'s comment header:

```ts
function genStyleDocs(s: Schema): string {
  let out = "<!-- GENERATED by tools/codegen.ts — do not edit -->\n\n";
  out += "# Styling\n\n";
  out += "`style` is an explicit, **non-web** prop on every widget. GTK styling is NOT web CSS: there is no `flex`, `grid`, `position`, `display`, or `justifyContent`. Layout comes from container widgets (`<box>`/`<grid>`), not from `style`. Unknown keys are rejected at the React renderer with a fix-it message and defensively rejected host-side.\n\n";
  if (s.style) {
    out += "## Valid keys\n\n| Key | Shape | Compiles to |\n|---|---|---|\n";
    for (const [k, def] of Object.entries(s.style.keys)) {
      const shape = def.kind === "object" ? "object (" + Object.keys(def.fields ?? {}).join(", ") + ")" : def.kind;
      const to = def.target === "widget" ? "widget margin properties" : "GTK CSS (`" + (def.css ?? "nested") + "`)";
      out += `| \`${k}\` | ${shape} | ${to} |\n`;
    }
    out += "\n";
  }
  out += "## Why `margin` differs from `padding`\n\nGTK widget **margins are widget properties** (`gtk_widget_set_margin_*`), not CSS — so `margin` compiles to `setMarginStart/End/Top/Bottom`. `padding` IS a GTK CSS property and stays in the generated `.nd-<id>` CSS block, alongside colors, fonts, and borders. (M5c-D2.)\n\n";
  out += "## React item-templates for ListView\n\nDeferred (M5c-D-followup): `<listview>` renders native string rows in v1. Item-template components need the synchronous bind design from spec D10.\n";
  return out;
}
```

- [ ] Wire the new writes at the bottom of the file (`:752-755`):

```ts
await writeIfChanged("docs/styling.md", genStyleDocs(schema));
```
and prepend `genStyleProp` output to `intrinsics.ts` via the `genIntrinsics` edit above (no separate write).

- [ ] Regenerate + build + typecheck + test:
```bash
nix develop -c bash -c 'bun tools/codegen.ts && zig build test && zig build && bun install --frozen-lockfile'
git diff --stat -- packages/react/src/generated src/generated docs
```
Expected: `intrinsics.ts` gains `StyleProp`/`styleKeySpec` + `style?` on every intrinsic; `widgets.zig` gains the two style tables; `docs/styling.md` created; `zig build` still compiles (the tables are unreferenced until Task 3 — Zig analyzes lazily); the protocol test passes. **Runtime unchanged** — M3/M4/M5b scripts still pass (spot-check one):
```bash
nix develop -c ./scripts/headless-m5b.sh
```

**Commit:**
```bash
git add schema/widgets.json src/protocol.zig tools/codegen.ts packages/react/src/generated/intrinsics.ts packages/react/src/generated/schema-meta.ts src/generated/widgets.zig docs/styling.md docs/widgets.md
git commit -m "feat: style schema + StyleProp codegen + generated styling docs"
```

### Interfaces (produced by this task)
- TS `StyleProp` type + `styleKeySpec` manifest (consumed by the Task 5 validator + every intrinsic).
- Zig `generated.style_keys` / `generated.style_subkeys` tables (consumed by `src/style.zig`, Task 3).
- `protocol.EventPayload.key` (consumed by the host style-error event).
- `docs/styling.md` (referenced by fix-it messages).

---

## TASK 3 — Host style compiler: `src/style.zig` + apply-path wiring

**Spine. Depends on: Task 2. Files: `src/style.zig` (NEW), `src/gtk_backend.zig`, `src/tree.zig`.**

Implements M5c-D1 (one provider per node, `nd-<id>` class, display-level, replace-on-update) and M5c-D2 (margin→widget props, rest→CSS). Compiles the `style` object using the generated tables — no per-key Zig.

- [ ] Create `src/style.zig`. It owns: the `id → *GtkCssProvider` map, the display-level install-once logic, the CSS-body builder walking `generated.style_keys`/`style_subkeys`, the margin-property application, and the unknown-key `ND_WARN` + style-error emit. Sketch (fill from the generated tables; keep it fail-loud):

```zig
const std = @import("std");
const gtk = @import("gtk");
const gdk = @import("gdk");
const gobject = @import("gobject");
const generated = @import("generated/widgets.zig");
const protocol = @import("protocol.zig");

var gpa: std.mem.Allocator = undefined;
var providers: std.AutoHashMapUnmanaged(u32, *gtk.CssProvider) = .empty;
pub const StyleErrorFn = *const fn (node_id: u32, key: []const u8) void;
var on_error: ?StyleErrorFn = null;
var ready = false;

pub fn init(allocator: std.mem.Allocator, err_fn: StyleErrorFn) void {
    gpa = allocator; on_error = err_fn; ready = true;
}

/// Called from tree.apply at create AND update whenever props.style is present.
pub fn applyStyle(widget: *gtk.Widget, node_id: u32, style: std.json.Value) void {
    if (style != .object) return;
    // 1. validate + split: unknown keys -> ND_WARN + styleError; margin -> widget; rest -> css.
    var css_buf: std.ArrayList(u8) = .empty;
    defer css_buf.deinit(gpa);
    var w = css_buf.writer(gpa);
    _ = w.print(".nd-{d} {{", .{node_id}) catch {};
    var it = style.object.iterator();
    var border_present = false;
    while (it.next()) |entry| {
        const key = entry.key_ptr.*;
        const def = findKey(key) orelse {
            std.debug.print("ND_WARN unknown style key \"{s}\" (GTK is not web CSS)\n", .{key});
            if (on_error) |f| f(node_id, key);
            continue;
        };
        if (std.mem.eql(u8, def.name, "margin")) {
            applyMargin(widget, entry.value_ptr.*);
        } else if (std.mem.eql(u8, key, "border")) {
            border_present = emitNested(&w, entry.value_ptr.*, "border") or border_present;
        } else if (std.mem.eql(u8, key, "font")) {
            _ = emitNested(&w, entry.value_ptr.*, "font");
        } else if (std.mem.eql(u8, key, "padding")) {
            emitSpacingCss(&w, "padding", entry.value_ptr.*);
        } else if (def.css) |css_name| {
            emitScalarCss(&w, css_name, entry.value_ptr.*, def.unit);
        }
    }
    if (border_present) _ = w.writeAll("border-style: solid;") catch {};
    _ = w.writeAll("}") catch {};

    // 2. install/replace this node's provider (M5c-D1).
    const css_z = gpa.dupeZ(u8, css_buf.items) catch return;
    defer gpa.free(css_z);
    const provider = providers.get(node_id) orelse blk: {
        const p = gtk.CssProvider.new();
        const display = gtk.Widget.getDisplay(widget);
        gtk.StyleContext.addProviderForDisplay(display, p.as(gtk.StyleProvider), 600); // _APPLICATION
        providers.put(gpa, node_id, p) catch {};
        var cls_buf: [32]u8 = undefined;
        const cls = std.fmt.bufPrintZ(&cls_buf, "nd-{d}", .{node_id}) catch "nd-x";
        gtk.Widget.addCssClass(widget, cls);
        break :blk p;
    };
    gtk.CssProvider.loadFromString(provider, css_z);
}
```

Fill in `findKey` (linear scan of `generated.style_keys`), `applyMargin` (int → all four `setMargin*`; object → per-side), `emitScalarCss` (`{css}: {value}{unit};` — colors/strings verbatim, ints get the unit), `emitSpacingCss` (int → `padding: Npx;`, object → four sides), `emitNested` (walk `generated.style_subkeys` where `parent == key`, emit each present sub-field; return `true` if any border field emitted). Cheap color sanity: a color value that isn't a string, or doesn't start with `#`/`rgb`, → `ND_WARN` + skip.

- [ ] In `src/gtk_backend.zig`, add the seam:
```zig
const style = @import("style.zig");

pub fn initStyle(sink_err: style.StyleErrorFn) void {
    style.init(arena, sink_err);
}
pub fn applyStyle(widget: *gtk.Widget, node_id: u32, style_value: std.json.Value) void {
    style.applyStyle(widget, node_id, style_value);
}
```
Call `initStyle` from wherever `setEventSink` is called (`src/runtime.zig:74` area — add a `backend.initStyle(&sendStyleErrorStatic)` next to `backend.setEventSink`). The style-error emitter mirrors `sendEventStatic`: `fn sendStyleErrorStatic(node_id: u32, key: []const u8) void { if (singleton) |s| s.sendEvent(node_id, "styleError", .{ .key = key }); }`.

- [ ] In `src/tree.zig` `apply`, invoke style at **create** (after `putMeta`, in the `create` arm ~:137) and at **update** (in the `update` arm ~:151, after `applyProps`):
```zig
if (op.props) |p| { if (p == .object) { if (p.object.get("style")) |st| backend.applyStyle(widget, op.id.?, st); } }
```
(one helper `applyStyleIfPresent(widget, id, op.props)` used in both arms keeps it DRY). The null backend path (`tree.zig` uses `backend.impl`) must no-op: add a `pub fn applyStyle(...) void {}` stub to `src/null_backend.zig` so the `backend` interface is uniform (conformance records style presence separately — Task 6).

- [ ] Add a Zig unit test for the CSS body builder (pure, no GTK display needed — factor the CSS-building half of `applyStyle` into a `pub fn compileCss(gpa, node_id, style) ![]u8` that `applyStyle` calls, so it's testable):
```zig
test "compileCss emits scoped block, splits margin out, rejects unknown key" {
    const gpa = std.testing.allocator;
    const parsed = try std.json.parseFromSlice(std.json.Value, gpa,
        "{\"background\":\"#fff\",\"padding\":8,\"margin\":4,\"flex\":1}", .{});
    defer parsed.deinit();
    const css = try compileCss(gpa, 7, parsed.value);
    defer gpa.free(css);
    try std.testing.expect(std.mem.indexOf(u8, css, ".nd-7 {") != null);
    try std.testing.expect(std.mem.indexOf(u8, css, "background-color: #fff;") != null);
    try std.testing.expect(std.mem.indexOf(u8, css, "padding: 8px;") != null);
    try std.testing.expect(std.mem.indexOf(u8, css, "margin") == null); // margin is a widget prop, not CSS
}
```
(`flex` triggers the ND_WARN path in `applyStyle`; `compileCss` may skip it silently or you can have `compileCss` collect rejected keys — keep the test to what `compileCss` returns.)

- [ ] Verify:
```bash
nix develop -c bash -c 'zig build test && zig build && ./scripts/headless-m5b.sh'
```
Expected: new CSS test passes; existing widgets (no `style`) still byte-identical (m5b green).

**Commit:**
```bash
git add src/style.zig src/gtk_backend.zig src/tree.zig src/null_backend.zig src/runtime.zig
git commit -m "feat: host style compiler (per-node css provider + margin props)"
```

### Interfaces (produced by this task)
- `backend.applyStyle(widget, id, style_value)` + `backend.initStyle(err_fn)` — called by `tree.apply`, error routed through `runtime.sendEvent("styleError")`.
- `style.compileCss` — pure, unit-tested; the CSS half of the compiler.

---

## TASK 4 — ListView widget: schema + codegen + host wiring

**Spine. Depends on: Task 3 (regenerates `widgets.zig`). Files: `schema/widgets.json`, `tools/codegen.ts`, `src/protocol.zig` (NodeMeta itemCount lives in tree, not protocol), `src/tree.zig`, `src/automation.zig`, regenerated generated files, `docs/widgets.md`.**

Implements M5c-D3 (self-contained scroller), D4 (itemCount in getTree), D6 (activate + SingleSelection). Native string rows only (D5).

### 4a — `schema/widgets.json`: the ListView object

- [ ] Insert after the `Grid` object (before `WebView`), so schema order stays stable:

```json
    {
      "name": "ListView",
      "intrinsic": "listview",
      "container": null,
      "props": [
        { "name": "items", "type": "stringList", "appliesTo": "createAndUpdate" },
        { "name": "selectedIndex", "type": "int", "default": -1, "appliesTo": "createAndUpdate" },
        { "name": "testID", "type": "string", "appliesTo": "meta" }
      ],
      "events": [
        { "name": "rowActivated", "ndpName": "onRowActivated", "payload": "index" }
      ],
      "automation": { "role": "list", "textFrom": null }
    },
```

`container: null` — ListView is data-driven; `items` is its content, not JSX children (D5). `selectedIndex` default `-1` = no selection (mapped to `GTK_INVALID_LIST_POSITION`).

### 4b — `tools/codegen.ts`: create + apply + signal + itemCount

- [ ] **Create body** (add a `w.name === "ListView"` arm in `genZigCreateBody` before the final `throw`, matching the emit style):

```ts
  } else if (w.name === "ListView") {
    out += "        const model = gtk.StringList.new(null);\n";
    out += "        if (propArray(props, \"items\")) |arr| {\n";
    out += "            for (arr.items) |item| { if (item == .string) gtk.StringList.append(model, dupeZ(item.string)); }\n";
    out += "        }\n";
    out += "        const selection = gtk.SingleSelection.new(model.as(gio.ListModel)); // transfer-full: selection owns model\n";
    out += `        const sel_idx = propInt(props, "selectedIndex") orelse ${dflt(w, "selectedIndex")};\n`;
    out += "        if (sel_idx >= 0) gtk.SingleSelection.setSelected(selection, @intCast(sel_idx));\n";
    out += "        const factory = gtk.SignalListItemFactory.new();\n";
    out += "        _ = gobject.signalConnectData(asObject(factory), \"setup\", @ptrCast(&lvSetup), null, null, .{});\n";
    out += "        _ = gobject.signalConnectData(asObject(factory), \"bind\", @ptrCast(&lvBind), null, null, .{});\n";
    out += "        const list = gtk.ListView.new(selection.as(gtk.SelectionModel), factory.as(gtk.ListItemFactory)); // transfer-full: list owns selection+factory\n";
    out += "        const sw = gtk.ScrolledWindow.new(); // M5c-D3: ListView needs a scroller\n";
    out += "        gtk.ScrolledWindow.setChild(sw, list.as(gtk.Widget));\n";
    out += "        return sw.as(gtk.Widget);\n";
```

- [ ] **`lvSetup`/`lvBind` callbacks** — add to a new `LISTVIEW_CALLBACKS` constant string emitted once in `genZigEvents` (they are not per-widget signal-table entries because they connect at create, not via `connectEvents`; emit them unconditionally when any widget named `ListView` exists, or always — keep simple, always emit):

```zig
fn lvSetup(_: *gobject.Object, list_item: *gtk.ListItem, _: ?*anyopaque) callconv(.c) void {
    const label = gtk.Label.new(null);
    gtk.ListItem.setChild(list_item, label.as(gtk.Widget));
}
fn lvBind(_: *gobject.Object, list_item: *gtk.ListItem, _: ?*anyopaque) callconv(.c) void {
    const child = gtk.ListItem.getChild(list_item) orelse return;
    const label: *gtk.Label = @ptrCast(@alignCast(child));
    const obj = gtk.ListItem.getItem(list_item) orelse return;
    const so: *gtk.StringObject = @ptrCast(@alignCast(obj));
    gtk.Label.setText(label, gtk.StringObject.getString(so));
}
```

- [ ] **Apply body** (`genZigApplyBody`, ListView arms). `items` update = splice-replace the model; `selectedIndex` = set on the selection. The tracked widget is the ScrolledWindow, so recover the inner ListView via `getChild`:

```ts
    } else if (w.name === "ListView" && p.name === "items") {
      out += "        if (propArray(props, \"items\")) |arr| {\n";
      out += "            const sw: *gtk.ScrolledWindow = @ptrCast(@alignCast(widget));\n";
      out += "            const list: *gtk.ListView = @ptrCast(@alignCast(gtk.ScrolledWindow.getChild(sw).?));\n";
      out += "            const selection: *gtk.SingleSelection = @ptrCast(@alignCast(gtk.ListView.getModel(list).?));\n";
      out += "            const model: *gtk.StringList = @ptrCast(@alignCast(gtk.SingleSelection.getModel(selection).?));\n";
      out += "            const n = gio.ListModel.getNItems(model.as(gio.ListModel));\n";
      out += "            gtk.StringList.splice(model, 0, n, null); // clear\n";
      out += "            for (arr.items) |item| { if (item == .string) gtk.StringList.append(model, dupeZ(item.string)); }\n";
      out += "        }\n";
    } else if (w.name === "ListView" && p.name === "selectedIndex") {
      out += "        if (propInt(props, \"selectedIndex\")) |idx| {\n";
      out += "            const sw: *gtk.ScrolledWindow = @ptrCast(@alignCast(widget));\n";
      out += "            const list: *gtk.ListView = @ptrCast(@alignCast(gtk.ScrolledWindow.getChild(sw).?));\n";
      out += "            const selection: *gtk.SingleSelection = @ptrCast(@alignCast(gtk.ListView.getModel(list).?));\n";
      out += "            if (idx >= 0) gtk.SingleSelection.setSelected(selection, @intCast(idx));\n";
      out += "        }\n";
```
(Verify `gtk.ListView.getModel`, `gtk.SingleSelection.getModel`, `gio.ListModel.getNItems` exist: `rg -n "gtk_list_view_get_model\b|gtk_single_selection_get_model\b|g_list_model_get_n_items\b" $G vendor/gobject-bindings/src/gio2/gio2.zig` — if `getModel` differs, keep a host-side `id → *StringList` map like `radio_groups` instead. Embed the rg result in the task before writing.)

- [ ] **Signal wiring** — add `ListView.rowActivated` to the `SIGNALS` table (`:487`) and a callback to `CALLBACK_BODIES`:

```ts
  "ListView.rowActivated": { signal: "activate", target: "widget", cb: "cbListActivate", suppress: false },
```
```zig
fn cbListActivate(obj: *gobject.Object, position: c_uint, data: ?*anyopaque) callconv(.c) void {
    const node_id: u32 = @intCast(@intFromPtr(data));
    _ = obj;
    if (emit) |f| f(node_id, "rowActivated", .{ .index = @intCast(position) });
}
```
The `activate` signal passes `(list_view, position, user_data)` — note the extra `position` arg (like `notify::` handlers have an extra arg). The generated `connectEvents` connects on the ScrolledWindow's child if the tracked widget is the scroller — **but** `connectEvents` receives the tracked widget (the ScrolledWindow). Add a ListView special-case in `genZigEvents` so it connects on the inner ListView: when `w.name === "ListView"`, emit `const target = gtk.ScrolledWindow.getChild(@ptrCast(@alignCast(widget))).?;` and connect on `asObject(target)` instead of `asObject(widget)`. Keep every other widget's wiring byte-identical.

### 4c — `src/tree.zig` + `src/automation.zig`: itemCount

- [ ] `NodeMeta` (`src/tree.zig:12`) gains `item_count: ?u32 = null`. In `apply`'s `create` arm, after `putMeta`, if the widget is a ListView read `props.items` length (or a future `itemCount`) and `metaGet(id).?.item_count = <len>`; in `update` refresh it when `items` changes. Add a `setMetaItemCount(id, n)` setter mirroring `setMetaText`.

- [ ] `src/automation.zig`: `JsonNode` (`:394`) gains `itemCount: ?u32 = null`. `buildNode` (`:447`) sets it from `meta.item_count`. Because ListView's recycled rows are untracked GTK children, `collectTrackedChildren` already yields `children: []` for it (M5b behavior) — no change needed there; just confirm in the drive that the ListView node has empty children + the itemCount.

- [ ] Regenerate + build + test:
```bash
nix develop -c bash -c 'bun tools/codegen.ts && zig build test && zig build && ./scripts/headless-m5b.sh'
git diff --stat -- src/generated docs/widgets.md
```
Expected: `widgets.zig` gains the ListView create/apply/callbacks; `docs/widgets.md` gains ListView; m5b still green (ListView unused by the m5b gallery).

**Commit:**
```bash
git add schema/widgets.json tools/codegen.ts src/tree.zig src/automation.zig src/generated/widgets.zig docs/widgets.md
git commit -m "feat: ListView widget (GtkListView + string-list factory, itemCount in getTree)"
```

### Interfaces (produced by this task)
- `<listview items={…} selectedIndex? onRowActivated?>` intrinsic; getTree `itemCount` on ListView nodes; `rowActivated {index}` event.

---

## TASK 5 — TS style validator with fix-it messages (the marquee feature)

**Fan-out. Depends on: Tasks 2, 4. Files: `packages/react/src/style-validate.ts` (NEW), `packages/react/src/host-config.ts`, a bun test.**

- [ ] Create `packages/react/src/style-validate.ts`. It consumes the generated `styleKeySpec` and throws a `StyleError` with a fix-it message on any unknown top-level or nested key:

```ts
import { styleKeySpec } from "./generated/intrinsics.ts";

export class StyleError extends Error {}

const validTop = Object.keys(styleKeySpec);

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
  const dp = Array.from({ length: m + 1 }, (_, i) => [i, ...Array(n).fill(0)]);
  for (let j = 0; j <= n; j++) dp[0][j] = j;
  for (let i = 1; i <= m; i++) for (let j = 1; j <= n; j++)
    dp[i][j] = Math.min(dp[i - 1][j] + 1, dp[i][j - 1] + 1, dp[i - 1][j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1));
  return dp[m][n];
}

function reject(bad: string, valid: string[], where: string): never {
  const near = nearest(bad, valid);
  const hint = near ? `Did you mean "${near}"?` : `Valid keys: ${valid.join(", ")}.`;
  throw new StyleError(`Invalid style key "${bad}"${where} — GTK styling is not web CSS. ${hint} See docs/styling.md`);
}

export function validateStyle(style: unknown): void {
  if (style == null || typeof style !== "object") return;
  for (const [k, v] of Object.entries(style as Record<string, unknown>)) {
    if (!(k in styleKeySpec)) reject(k, validTop, "");
    const nested = styleKeySpec[k];
    if (nested && v != null && typeof v === "object" && !Array.isArray(v)) {
      for (const nk of Object.keys(v as Record<string, unknown>)) {
        if (!nested.includes(nk)) reject(nk, nested, ` in "${k}"`);
      }
    }
  }
}
```

- [ ] Wire it into `host-config.ts` at BOTH emission points, so both create and update validate before the op ships. In `emitCreateIfNew` (`:51`), right after `const props = { ...inst.props }`, add `if ("style" in props) validateStyle(props.style);`. In `commitUpdate` (`:126`), before pushing the update op, add `if ("style" in newProps) validateStyle(newProps.style);`. Import `validateStyle` at the top. (A thrown `StyleError` surfaces through the reconciler's error handler → the renderer's `(e) => { throw e; }` at `renderer.ts:42` → process exits with the fix-it message on stderr, which the drive asserts.)

- [ ] Add a bun test `packages/react/src/style-validate.test.ts`:

```ts
import { expect, test } from "bun:test";
import { validateStyle, StyleError } from "./style-validate.ts";

test("accepts valid style", () => {
  validateStyle({ background: "#fff", padding: 8, font: { fontSize: 14, fontWeight: "bold" } });
});
test("rejects display with fix-it", () => {
  expect(() => validateStyle({ display: "flex" })).toThrow(StyleError);
  try { validateStyle({ display: "flex" }); } catch (e) {
    expect((e as Error).message).toContain("GTK styling is not web CSS");
    expect((e as Error).message).toContain("docs/styling.md");
  }
});
test("suggests nearest key for a typo", () => {
  try { validateStyle({ colour: "#000" }); } catch (e) {
    expect((e as Error).message).toContain('Did you mean "color"');
  }
});
test("rejects unknown nested font key", () => {
  expect(() => validateStyle({ font: { fontStyle: "italic" } })).toThrow(StyleError);
});
```

- [ ] Verify:
```bash
nix develop -c bash -c 'cd packages/react && bun test src/style-validate.test.ts'
```
Expected: 4 tests pass.

**Commit:**
```bash
git add packages/react/src/style-validate.ts packages/react/src/style-validate.test.ts packages/react/src/host-config.ts
git commit -m "feat(react): style validator with fix-it messages rejecting web CSS"
```

### Interfaces (produced by this task)
- `validateStyle(style)` throwing `StyleError` — the agent-facing fix-it path, asserted by the drive (Task 9).

---

## TASK 6 — Conformance: style round-trip + ListView itemCount

**Fan-out. Depends on: Tasks 3, 4. Files: `src/conformance.zig`, `src/null_backend.zig`.**

- [ ] Ensure the null backend records a `style` prop generically (it already records unknown props as canonical strings — confirm `style` doesn't hit a special case; the `applyStyle` no-op stub from Task 3 keeps the interface uniform). Add a conformance test that a widget created with a `style` prop round-trips the style value in the null backend's recorded props (proves `style` rides `props` and isn't dropped):

```zig
test "style prop rides create props (null backend round-trip)" {
    const gpa = std.testing.allocator;
    try nb.init(gpa, schema_json);
    defer nb.deinitAll();
    const parsed = try std.json.parseFromSlice(std.json.Value, gpa,
        "{\"style\":{\"background\":\"#fff\",\"padding\":8}}", .{});
    defer parsed.deinit();
    const node = try nb.createWidget(dummyApp(), "Label", parsed.value);
    try std.testing.expect(node.props.get("style") != null);
}
```

- [ ] Add a conformance test that ListView is in the schema-driven create loop with its `items`/`selectedIndex`/`rowActivated` surface (it falls out of the generic loops automatically — assert it's present so a regression that drops it fails loudly):

```zig
test "ListView is schema-driven with items + rowActivated event" {
    const gpa = std.testing.allocator;
    try nb.init(gpa, schema_json);
    defer nb.deinitAll();
    const node = try nb.createWidget(dummyApp(), "ListView", null);
    nb.connectEvents(node, "ListView");
    try std.testing.expect(hasEvent(node, "rowActivated"));
}
```
(reuse/extend whatever event-assertion helper conformance already has; if none, scan `node.events`.)

- [ ] Verify:
```bash
nix develop -c zig build test
```
Expected: all conformance tests pass (no display server).

**Commit:**
```bash
git add src/conformance.zig src/null_backend.zig
git commit -m "test: conformance for style round-trip + ListView schema surface"
```

---

## TASK 7 — Gallery: styled section + ListView tab + revert createElement

**Fan-out. Depends on: Tasks 1, 4, 5. Files: `examples/gallery/main.tsx`.**

- [ ] Revert the `createElement` workaround (`examples/gallery/main.tsx:2,61`) to plain JSX now that `key` types (Task 1): remove the `createElement` import; change the log-scroll rows to `{Array.from({ length: 40 }, (_, i) => <label key={i} text={`Row ${i}`} />)}`.

- [ ] Add a **styled tab** to the existing `<tabview>` proving colors/padding/border/margin round-trip, plus a **deliberate invalid-style negative test** gated behind a prop so it only mounts when the drive asks (mounting an invalid style throws and would crash the whole app on load — so gate it behind an env flag the drive sets in a SECOND host process, OR keep the invalid style in the drive itself via a to-be-mounted node). **Simplest correct approach:** the invalid-style assertion is driven by a SEPARATE tiny script `examples/gallery-badstyle.tsx` that mounts one `<label style={{ display: "flex" }} />` and is expected to crash with the fix-it message — the headless script runs it and greps the fix-it string (see Task 9). The main gallery only carries VALID styles:

```tsx
          <box tabLabel="Styled" orientation="vertical" spacing={6} testID="styled-tab">
            <label testID="styled-label" text="Styled label"
              style={{ background: "#2266cc", color: "#ffffff", padding: 8, margin: 4,
                       font: { fontSize: 16, fontWeight: "bold" },
                       border: { borderWidth: 2, borderColor: "#003399", borderRadius: 6 } }} />
            <button testID="styled-button" label="Styled button"
              style={{ background: "#cc2222", color: "#ffffff", padding: 6 }} />
          </box>
```

- [ ] Add a **ListView tab** with 100k rows from React state, mounted via one props update:

```tsx
          <listview tabLabel="List" testID="big-list"
            items={rows}
            selectedIndex={selectedRow}
            onRowActivated={(e) => setActivatedRow(e.index)} />
          {/* rows = useMemo(() => Array.from({ length: 100000 }, (_, i) => `Item ${i}`), []) */}
          {/* plus a <label testID="activated-label" text={`Activated: ${activatedRow}`} /> outside the tab */}
```
Add the `rows`/`selectedRow`/`activatedRow` state at the top of `App`. Put `<label testID="activated-label" text={`Activated: ${activatedRow}`} />` in the always-visible column so the drive's `waitFor` can see it.

- [ ] Typecheck the gallery:
```bash
nix develop -c bash -c 'cd examples/gallery && bun x tsc --noEmit -p tsconfig.json'
```
Expected: clean (styled props type-check against `StyleProp`; `key` type-checks; ListView props type-check).

**Commit:**
```bash
git add examples/gallery/main.tsx examples/gallery-badstyle.tsx
git commit -m "feat(gallery): styled tab + 100k-row ListView tab; revert createElement to jsx"
```

---

## TASK 8 — (folded into Task 2)

`docs/styling.md` is codegen-emitted (Task 2). No standalone task — this slot is intentionally empty so the numbering matches the parallelism note. If the self-review finds `docs/styling.md` needs a hand-authored intro beyond the generated content, add it as a `docs:` commit with an explicit path.

---

## TASK 9 — Drive script + headless script + CI

**Depends on: Tasks 5, 6, 7. Files: `scripts/m5c-drive.ts` (NEW), `scripts/headless-m5c.sh` (NEW), `.github/workflows/ci.yml`.**

- [ ] Create `scripts/m5c-drive.ts` (mirror `scripts/m5b-drive.ts`'s `AutomationClient`/`find`/`waitFor` scaffold). Assertions:

```ts
// 1. Styled widget renders: its node is present + a screenshot round-trips.
const styled = mustFind("styled-label");
if (styled.type !== "Label") throw new Error("styled-label wrong type");
// (color/border are visual; the screenshot below is the proof they compiled without error.)

// 2. ListView present with 100k items; getTree reports itemCount, NOT 100k children.
const list = mustFind("big-list");
if (list.type !== "ListView") throw new Error("big-list not a ListView");
if ((list as any).itemCount !== 100000) throw new Error(`itemCount=${(list as any).itemCount}, want 100000`);
if (list.children.length !== 0) throw new Error(`ListView dumped ${list.children.length} children (must be 0)`);

// 3. Scroll the list (ListView is a ScrolledWindow, M5c-D3).
const scrolled = (await client.call("scroll", { ref: list.ref, dy: 5000 })) as { x: number; y: number };
if (!(scrolled.y > 0)) throw new Error(`list did not scroll (y=${scrolled.y})`);

// 4. rowActivated round-trip: setValue selects; a real activate needs the signal —
//    drive it by setValue on selectedIndex then asserting selection, and (if the host
//    exposes an activate action) activating row 3. v1: assert selection via getTree re-read,
//    OR emit activate through a click on the row is not addressable (rows untracked) —
//    so assert onRowActivated by having the gallery pre-select and the drive re-read.
//    Minimal honest assertion: selection is settable and itemCount stable after scroll.
const list2 = mustFind("big-list"); // re-read
if ((list2 as any).itemCount !== 100000) throw new Error("itemCount changed after scroll");
```

For `rowActivated`: the row widgets are untracked (recycled), so the drive can't click a row by ref. To exercise the `activate` signal end-to-end, add a small **`activate` automation action** is out of scope; instead assert `rowActivated` wiring exists via conformance (Task 6) and prove the selection path via `selectedIndex` (a `setValue`-style path). **If** a lightweight `activateRow` RPC is desired, note it as a follow-up in the drive comment — do NOT expand automation scope in M5c. Keep the drive's rowActivated coverage to: the event is wired (conformance) + the gallery's `activated-label` starts at `-1` and the ListView mounted with `selectedIndex` shows the selection is applied (screenshot).

- [ ] Invalid-style negative test — run `examples/gallery-badstyle.tsx` as a SECOND short-lived host and assert it dies with the fix-it message (this is the marquee assertion):
  The headless script runs a second host with `ND_SCRIPT=examples/gallery-badstyle.tsx`, captures its stderr, and greps:
```
Invalid style key "display" — GTK styling is not web CSS
```
  plus `docs/styling.md`. If the string is absent (the app didn't reject), FAIL.

- [ ] Add the styled-widget screenshot assertion (poll like m5b, `-32603`-retry) and emit `M5C_DRIVE_OK` at the end.

- [ ] Create `scripts/headless-m5c.sh` (mirror `scripts/headless-m5b.sh`; weston socket `nd-headless-m5c`, `ND_SCRIPT=examples/gallery/main.tsx`). Two host phases: (A) main gallery + `m5c-drive.ts` → `M5C_DRIVE_OK` + PNG check; (B) a second host with `ND_SCRIPT=examples/gallery-badstyle.tsx`, capture stderr to a log, `grep -q 'Invalid style key "display"'` and `grep -q 'docs/styling.md'` → FAIL if missing. Both phases must pass.

```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
export WAYLAND_DISPLAY=nd-headless-m5c
export GSK_RENDERER=cairo
export GDK_BACKEND=wayland
export NATIVE_AUTOMATION=1
weston --backend=headless --socket="$WAYLAND_DISPLAY" --idle-time=0 &
WESTON_PID=$!
trap 'kill "$WESTON_PID" 2>/dev/null || true; kill "$HOST_PID" 2>/dev/null || true' EXIT
for _ in $(seq 1 50); do [ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ] && break; sleep 0.1; done

# Phase A: styled + ListView gallery drive.
LOG=$(mktemp)
ND_SCRIPT=examples/gallery/main.tsx ./zig-out/bin/nd-hello >"$LOG" 2>&1 &
HOST_PID=$!
for _ in $(seq 1 120); do grep -q "ND_AUTOMATION_LISTENING" "$LOG" && grep -q "ND_COMMIT_APPLIED" "$LOG" && break; sleep 0.1; done
grep -q "ND_AUTOMATION_LISTENING" "$LOG" || { echo "FAIL: no automation listener"; cat "$LOG"; exit 1; }
SOCK=$(grep -m1 "ND_AUTOMATION_LISTENING" "$LOG" | sed 's/.*path=//')
ND_AUTOMATION_SOCKET="$SOCK" ND_SHOT_PATH="$XDG_RUNTIME_DIR/m5c-shot.png" bun scripts/m5c-drive.ts >"$XDG_RUNTIME_DIR/drive.log" 2>&1 \
  || { echo "FAIL: driver"; cat "$XDG_RUNTIME_DIR/drive.log"; cat "$LOG"; exit 1; }
cat "$XDG_RUNTIME_DIR/drive.log"
grep -q "M5C_DRIVE_OK" "$XDG_RUNTIME_DIR/drive.log" || { echo "FAIL: driver"; exit 1; }
[ -s "$XDG_RUNTIME_DIR/m5c-shot.png" ] && file "$XDG_RUNTIME_DIR/m5c-shot.png" | grep -q "PNG image" || { echo "FAIL: png"; exit 1; }
kill -TERM "$HOST_PID"; wait "$HOST_PID" 2>/dev/null || true

# Phase B: web-CSS rejection must crash loudly with the fix-it message.
BADLOG=$(mktemp)
ND_SCRIPT=examples/gallery-badstyle.tsx ./zig-out/bin/nd-hello >"$BADLOG" 2>&1 || true
grep -q 'Invalid style key "display"' "$BADLOG" || { echo "FAIL: no fix-it rejection"; cat "$BADLOG"; exit 1; }
grep -q 'docs/styling.md' "$BADLOG" || { echo "FAIL: fix-it missing docs pointer"; cat "$BADLOG"; exit 1; }

echo "headless m5c: OK (styling round-trip + web-CSS rejection + 100k ListView)"
```
```bash
chmod +x scripts/headless-m5c.sh
```

- [ ] Append the CI step after `headless m5b` (`.github/workflows/ci.yml:33-34`):
```yaml
      - name: headless m5c
        run: nix develop -c ./scripts/headless-m5c.sh
```

- [ ] Verify locally:
```bash
nix develop -c bash -c 'zig build && bun install --frozen-lockfile && ./scripts/headless-m5c.sh'
```
Expected tail: `M5C_DRIVE_OK …` then `headless m5c: OK (…)`. Debugging ladder: (1) `NDP_TRACE=1` in the host env → read the `styleError` Event frames; (2) if the styled screenshot is empty, the CSS provider install failed — check `ND_WARN` for a `parsing-error`; (3) if the ListView itemCount is wrong, the `items` splice path diverged — check `getTree`.

**Commit:**
```bash
git add scripts/m5c-drive.ts scripts/headless-m5c.sh .github/workflows/ci.yml
git commit -m "test: headless m5c drive (styling + web-CSS rejection + 100k ListView) + ci gate"
```

### Interfaces (produced by this task)
- `M5C_DRIVE_OK` marker; CI step `headless m5c`; the drive is the end-to-end proof for styling round-trip (M5c-D1/D2), web-CSS rejection (M5c-D7), and 100k-row ListView + itemCount (M5c-D3/D4).

---

## TASK 10 — Full-gate integration + docs/memory sync + self-review

**Depends on: Tasks 6, 9 (everything). Files: `CLAUDE-activeContext.md` (update in place, NEVER commit), nothing else unless self-review finds gaps.**

- [ ] Update the **full gate** line in `CLAUDE-activeContext.md` to append `docs/styling.md` to the freshness path and `./scripts/headless-m5c.sh` to the tail:
```bash
nix develop -c bash -c 'bun tools/codegen.ts && git diff --exit-code -- schema packages/react/src/generated src/generated docs/widgets.md docs/styling.md && zig build test && zig build && bun install --frozen-lockfile && ./scripts/headless-smoke.sh && ./scripts/headless-m2.sh && ./scripts/kill9-test.sh && ./scripts/headless-m3.sh && ./scripts/headless-m4.sh && ./scripts/headless-m5b.sh && ./scripts/headless-m5c.sh'
```
Add an **M5c done & green** entry to the State section covering: shared `style` prop → GtkCssProvider per node (`nd-<id>`, display-level, M5c-D1) + margin→widget-props split (M5c-D2); dual-layer web-CSS rejection with fix-it messages (M5c-D7); `docs/styling.md` generated; ListView (GtkListView + StringList factory, native string rows, self-scrolling M5c-D3, itemCount in getTree M5c-D4, activate + SingleSelection M5c-D6); JSX.IntrinsicAttributes/`key` codegen fix; gallery styled tab + 100k-row ListView tab; `headless-m5c` in CI. Note the sole deferral: **M5c-D-followup — React item-templates for ListView** (synchronous-bind design, spec D10). Mark spec §14 **M5 complete**.

- [ ] Run the ENTIRE updated gate and read every line:
```bash
nix develop -c bash -c 'bun tools/codegen.ts && git diff --exit-code -- schema packages/react/src/generated src/generated docs/widgets.md docs/styling.md && zig build test && zig build && bun install --frozen-lockfile && ./scripts/headless-smoke.sh && ./scripts/headless-m2.sh && ./scripts/kill9-test.sh && ./scripts/headless-m3.sh && ./scripts/headless-m4.sh && ./scripts/headless-m5b.sh && ./scripts/headless-m5c.sh'
```
Expected: codegen no-diff; all unit + conformance tests pass; all seven headless scripts pass.

- [ ] Hygiene:
```bash
git status --porcelain | rg "node_modules|zig-cache|.zig-cache|zig-out|CLAUDE" || echo "clean"
```
Expected: `clean` (CLAUDE-activeContext.md may show modified — leave it unstaged).

**No commit for this task** unless self-review forces fixes (then smallest `fix:` commits, explicit paths).

---

## Self-review checklist (run before declaring M5c done)

- [ ] **Regression: no-style is byte-identical.** `rg -n "applyStyle|compileCss" src/tree.zig` shows style is invoked ONLY when `props.style` is present; m3/m4/m5b headless scripts green at every task boundary and in the full gate.
- [ ] **D6 respected for the style surface.** `src/style.zig` contains NO per-widget code — it walks `generated.style_keys`/`style_subkeys` for every key. Per-widget ListView code exists ONLY in `src/generated/widgets.zig`. `rg -n "gtk\.(ListView|SignalListItemFactory|StringList|SingleSelection|CssProvider)\." src/ --glob '!src/generated/*' --glob '!src/style.zig'` → hits only where cross-cutting host code legitimately dispatches (automation), never per-widget bindings.
- [ ] **Determinism + freshness.** `bun tools/codegen.ts` twice → `git diff --exit-code` clean on all generated paths incl. `docs/styling.md`; CI freshness step green (path list updated to include `docs/styling.md`).
- [ ] **Codegen still fails loudly.** A style key with a bogus `kind`, or a ListView event without a SIGNALS template, throws at codegen (same fail-loud contract). Spot-check by adding a scratch bad key and confirming the throw; revert.
- [ ] **Fix-it is real, not aspirational.** The headless-m5c Phase B asserts the EXACT message `Invalid style key "display" — GTK styling is not web CSS` + `docs/styling.md` on stderr; the bun test asserts the nearest-key suggestion (`colour → color`). Both green.
- [ ] **Margin/CSS split proven.** `compileCss` unit test asserts `margin` is NOT in the CSS body and `padding` IS; a styled widget with margins renders (screenshot non-empty).
- [ ] **ListView is honest about scale.** getTree returns `itemCount: 100000` and `children: []` for the ListView; the drive fails if children are dumped. Screenshot non-empty proves the recycler rendered under cairo headless. Memory sanity: note in the drive comment that ~200 live row widgets back 100k items (research fact) — no per-row allocation on the host.
- [ ] **`key` types on intrinsics.** Gallery uses plain `<label key={i} …>` (no `createElement`); `bun x tsc --noEmit` on the gallery is clean.
- [ ] **Host defensive layer works even without the TS validator.** A `styleError` Event fires host-side for an unknown key (proven by feeding a bad key past TS — e.g. a `// @ts-expect-error` styled node in a scratch script, or trust the `gallery-badstyle` path which the TS validator catches first; add a host-only test if the TS layer masks it: temporarily bypass `validateStyle` and confirm `ND_WARN unknown style key`).
- [ ] **Explicit-path commits only; `CLAUDE*.md` never committed; lockfile change (if any) committed once.**
- [ ] **Deferral documented.** M5c-D-followup (React item-templates) is in `docs/styling.md`, `docs/widgets.md` (ListView note), and CLAUDE-activeContext. No half-built async-bind code left in the tree: `rg -n "item.?template|setup.*bind.*unbind" src packages` → only the native `lvSetup`/`lvBind` callbacks.
- [ ] **Spec §14 M5 complete.** Styling (validated, rejecting web CSS) ✓; GtkListView 100k-row recycling ✓; ~20 widgets on GTK ✓ (18 + ListView = 19, WebView stub; Menu remains explicitly deferred to a later milestone — note it).
