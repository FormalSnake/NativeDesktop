# M4 — Automation Layer v1: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Parallelism note.** Two file-disjoint tracks:
> - **Zig automation-server track (Tasks 1–6):** the node-metadata registry, the automation server thread + framed JSON-RPC dispatch on a second socket, the four RPC handlers (`getTree`/`screenshot`/`click`/`waitFor`) with UI-thread marshaling, the not-implemented stubs, and the `testID` plumbing on the host side. Touches only `src/*.zig`, `build.zig`.
> - **JS + MCP track (Tasks 7–8):** the `testID` intrinsic prop through `packages/react` + the counter demo, and the new `packages/mcp` stdio MCP wrapper. Touches only `packages/react/`, `examples/counter/`, `packages/mcp/`, root `package.json`/`bun.lock`.
> These two tracks share **no files**. **Task 9 (headless-m4 demo + SLO test + CI) integrates both and must run last.** Within the Zig track, Task 2 (server thread + dispatch) depends on Task 1 (registry); Tasks 3–6 (handlers) depend on Task 2 and are mutually parallel. The `testID` change spans both tracks: the host-side storage is Task 1; the JS-side intrinsic prop is Task 7 — they meet at the wire (`props.testID`) and are independently testable.

**Goal:** From the M3 React-drives-GTK host, ship the agent-first automation layer v1 (spec §8, D11). Behind `NATIVE_AUTOMATION=1`, the Zig host opens a **second** unix socket at `$XDG_RUNTIME_DIR/nd-automation-<pid>.sock` and serves framed **JSON-RPC 2.0** (u32 LE length prefix + UTF-8 JSON, exactly the NDP outer framing) to a single connected client. Four methods work against the live retained widget tree — `getTree` (structured snapshot with stable refs, widget types, `testID`, text, visibility, and logical-unit geometry with an explicit coordinate-space contract), `screenshot` (in-process GTK→GSK render to a PNG file), `click` (semantic dispatch with a Playwright-style actionability hit-test first), and `waitFor` (UI-thread tree polling) — while `setValue`/`type`/`scroll` return a structured "not implemented until M5 widgets" error. A first-party **`packages/mcp`** Bun stdio MCP server wraps the socket as agent tools (`nd_get_tree`, `nd_screenshot`, `nd_click`, `nd_wait_for`). A headless demo (`scripts/headless-m4.sh` + a dependency-light Bun driver) drives the counter end to end via the raw socket, and the **D11 SLO test** SIGSTOPs the Bun child and asserts `getTree`+`screenshot` still answer within 1s.

**Architecture:** Unchanged two-process topology (spec D1). The M2/M3 host owns the authoritative retained tree on the GTK/GLib main thread and applies `CommitBatch`es via `marshalCommit`→`invokeFull`→`tree.apply` (`src/runtime.zig`). M4 adds a **third** thread (alongside the M2 NDP reader thread) — an automation listener that accepts **one** client at a time on the second socket and answers JSON-RPC requests. Every request that touches widgets marshals to the GTK main thread via the **same `glib.MainContext.default().invokeFull(...)` pattern the runtime already uses**, and blocks the automation thread on the response with `std.Io.Mutex` + `std.Io.Condition` (the exact `std.Io` primitives already present in `runtime.zig`). Tree access happens **only on the UI thread** — no new mutexes around the tree; the automation thread never calls GTK directly. This is what lets `getTree`/`screenshot` answer even when the Bun child is stalled: those two operations only read GTK state the main thread already holds, and the main GLib loop keeps ticking regardless of the child (D11).

**Owner-decided spec deviation (record it):** the v1 automation transport is **framed JSON-RPC over a second unix socket**, not the WebSocket + stdio that spec §8 names. WebSocket is deferred (a later milestone). Rationale: the host already has a proven length-prefixed-JSON unix-socket stack (NDP framing, `encodeFrame`, the reader-thread accept loop); reusing it costs a day and zero new dependencies, versus hand-rolling an HTTP/WS upgrade + frame masking in Zig. Agents reach the server through the MCP wrapper (`packages/mcp`), which speaks stdio MCP outward and the framed unix socket inward — so the "stdio + MCP" half of the spec surface is satisfied, only the wire between MCP and host changed. **Task 9 Step 6 appends a one-paragraph note to the design spec §8 recording this deviation.**

**Tech Stack:** Zig 0.16.0 (exact), the vendored zig-gobject bindings at `vendor/gobject-bindings` (glib2/gobject2/gio2/gtk4/gsk4/gdk4/graphene1), Bun 1.3.13 (from the flake), `@modelcontextprotocol/sdk` (verified `1.29.0` this session — re-verify at implement-time) for `packages/mcp`, GTK4 ≥ 4.20 (devshell 4.22.4), GSK cairo renderer under weston headless for CI. TypeScript strict; `bunx tsc --noEmit`.

## Global Constraints

Carried over from M1/M2/M3, unchanged:

- Zig is exactly `0.16.0`; `build.zig`'s `checkZigVersion()` guard stays.
- Bun is pinned `1.3.13` (flake devshell).
- No `@cImport` anywhere; all GTK/GSK/GDK/graphene access goes through the vendored zig-gobject modules imported as `glib`/`gobject`/`gio`/`gtk` (and the new `gsk`/`gdk`/`graphene` imports M4 adds to `build.zig`).
- No hand-written per-widget C bindings (spec D6); only the vendored generated modules.
- Headless CI uses `weston --backend=headless` + `GSK_RENDERER=cairo` — NOT Broadway, NOT Xvfb/X11. Each headless script uses a **unique weston socket name**.
- Commit style: short imperative lowercase subject (e.g. `feat: automation server thread + json-rpc dispatch`). No co-author trailers, no body unless closing an issue.
- All commands run inside the devshell (direnv activates it; in CI, `nix develop -c`).
- Host prints machine-greppable markers to **stderr**. M4 adds: `ND_AUTOMATION_LISTENING path=<sock>`, `ND_AUTOMATION_CONNECTED`, `ND_AUTOMATION_DISCONNECTED`, `ND_RPC method=<m> id=<n>`. Scripts capture `2>&1`.
- TypeScript is strict. New package code under `packages/mcp/` must pass `bunx tsc --noEmit`.
- `git add` explicit paths per task — never `git add -A`. Confirm `node_modules/` is never staged.

### M4-new constraints (owner decisions, verbatim)

- **Enablement.** The automation server starts **only** when `NATIVE_AUTOMATION=1` is set in the host's environment. When unset, no second socket is opened and no thread spawns — zero cost for production builds that don't opt in. (Spec §8: "always-on in dev; opt-in flag for production builds"; M4 implements the flag, dev-always-on is a CLI concern for M8.)
- **Transport.** Second unix socket at `$XDG_RUNTIME_DIR/nd-automation-<pid>.sock` (fall back to `/tmp` if `XDG_RUNTIME_DIR` unset, mirroring `runtime.zig`'s NDP socket). Framing is the **existing NDP outer frame**: `u32 LE length ‖ UTF-8 JSON`. The JSON payload is a **JSON-RPC 2.0** message (`{"jsonrpc":"2.0","id":…,"method":…,"params":…}` request; `{"jsonrpc":"2.0","id":…,"result":…}` or `{"jsonrpc":"2.0","id":…,"error":{"code":…,"message":…,"data":…}}` response). Reuse `protocol.encodeFrame` for outbound frames and the `readFrame` length-prefix loop for inbound.
- **One client at a time.** The listener `accept()`s a single client, serves it until disconnect, then loops back to `accept()` for the next. No concurrent clients (the MCP wrapper is the sole intended client).
- **UI-thread marshaling + blocking.** Every handler that touches widgets builds a request struct, `invokeFull`s a callback onto the GLib main context, and blocks the automation thread on an `std.Io.Condition` (paired with an `std.Io.Mutex`) until the UI callback signals completion. The UI callback runs `tree`/backend reads on the main thread and writes the result into the shared request struct. **No new mutex guards the tree itself** — the tree is only ever read on the UI thread; the mutex/condition guard only the request/response handoff.
- **Tree metadata registry (host-side).** `src/tree.zig` today stores only `nodes: AutoHashMapUnmanaged(u32, *gtk.Widget)`. M4 adds a parallel `meta: AutoHashMapUnmanaged(u32, NodeMeta)` capturing, per node: `widget_type` (owned copy of the `"Window"|"Box"|"Label"|"Button"` string), `test_id` (optional owned copy of `props.testID`), `text` (optional owned copy of the label text / button label), `parent` (u32, `0` = root), and insertion order sufficient to reconstruct the nested tree. Populated in `tree.apply` alongside `nodes.put`; freed on `remove`. This is the snapshot source of truth — `getTree` reads `meta` + live GTK geometry, never re-parses props.
- **`testID` prop.** Every widget accepts an optional `testID: string` prop. It is plumbed JS-side (Task 7: intrinsic prop types → props JSON, already flows through the existing `create`/`update` op `props`), stored host-side in `NodeMeta.test_id` (Task 1), and echoed in `getTree` — **it is never applied to the GTK widget** (no `gtk_widget_set_name` call in M4; that is an M5+ a11y-mirroring concern). Strip `testID` from the prop set handed to `backend.createWidget`/`applyProps` so it is not mistaken for a widget property.
- **RPC surface v1 (exact methods + result shapes below).**
- **`screenshot` in-process render.** Verified GTK/GSK/GDK path (symbols confirmed this session, table below): get the window's live renderer via `gtk.Native.getRenderer`, snapshot the widget subtree to a `gsk.RenderNode`, `gsk.Renderer.renderTexture` to a `gdk.Texture`, `gdk.Texture.saveToPng(path)`. Writes to a **caller-supplied path**; returns `{path, width, height}`. No base64 in v1.
- **`click` actionability.** Before dispatching, run the hit-test: the ref must exist in the registry, be visible (`gtk.Widget.getVisible`), be mapped (`gtk.Widget.getMapped`), and its computed bounds (via `gtk.Widget.computeBounds` to the window root) must contain its own center point. Fail with a structured JSON-RPC error (`code -32001`, `message "not actionable"`, `data {reason}`) if any check fails. Dispatch is **semantic**: for a Button, emit `clicked` (verified emit path below); GTK4 removed synthetic events (research gotcha) so this is the only correct Linux model.
- **Not-implemented stubs.** `setValue`, `type`, `scroll` are dispatched but return JSON-RPC error `code -32601` (method-not-found style) with `message "not implemented until M5 widgets"` — the signatures are defined now so the MCP surface and clients are stable; only `window`/`box`/`label`/`button` exist in M4.
- **MCP wrapper decision.** Use **`@modelcontextprotocol/sdk`** (npm, verified `1.29.0` present this session), not a hand-roll — one line of justification: the SDK's `McpServer` + `StdioServerTransport` gives a spec-correct stdio MCP handshake and tool schema/validation for free, and the socket client is the only bespoke code; hand-rolling the MCP framing to save one dependency is negative-value for an agent-facing deliverable. `packages/mcp` connects to the socket named by `ND_AUTOMATION_SOCKET`.
- **Demo + SLO.** `scripts/headless-m4.sh` launches weston-headless + the counter with `NATIVE_AUTOMATION=1`, then runs a **dependency-light** Bun driver (`scripts/m4-drive.ts`) that talks the socket **directly** (not through MCP): `getTree`, find the button by `testID`, `click` ×3, `waitFor` label text `Clicks: 3`, `screenshot` to a file, assert the PNG exists and is non-empty. Then the **D11 SLO** sub-test: SIGSTOP the bun child, assert `getTree`+`screenshot` still answer within 1s, SIGCONT, exit clean. CI appends one step.
- **Counter demo.** `examples/counter/main.tsx` gains `testID` props (minimal diff): `testID="increment-button"` on the button, `testID="clicks-label"` on the clicks label.

### Landed-code reality (authoritative — read before writing)

| Fact | Landed reality (file:line) |
|---|---|
| Second socket must not collide with NDP socket | NDP socket is `nd-<pid>.sock` (`src/runtime.zig:53`); automation socket is `nd-automation-<pid>.sock` — distinct basename. |
| Socket + net API | `std.Io.net.UnixAddress.init(path)` → `.listen(io, .{})` → `server.accept(io)` (`src/runtime.zig:56,57,103`). Reuse verbatim. |
| Framing helpers | `protocol.encodeFrame(gpa, value)` (u32 LE len + JSON) and the `readFrame` length-prefix reader (`src/runtime.zig:164`). `readFrame` is a `Runtime` method today — M4's automation thread needs its own copy (or a free function); factor a `protocol.readFrame(io, reader, gpa)` if convenient, else inline. |
| Marshal-to-UI pattern | `glib.MainContext.default().invokeFull(glib.PRIORITY_DEFAULT, &cb, data, null)` (`src/runtime.zig:183`); callback returns `G_SOURCE_REMOVE` (=0). Reuse this exact shape; M4's callback additionally signals a condition. |
| `std.Io.Mutex` usage | `.init` default, `.lockUncancelable(io)`/`.unlock(io)` (`src/runtime.zig:21,94`). `self.* = undefined` skips field defaults — set `.init` explicitly (hard-won fact, activeContext). |
| Tree storage | `nodes: AutoHashMapUnmanaged(u32, *gtk.Widget)`, `generation: u32` only — `src/tree.zig:9`. **No type/testID/text/parent stored today** — Task 1 adds `meta`. |
| `tree.apply` create arm | `backend.createWidget(app, op.widget.?, op.props)` then `nodes.put(gpa, op.id.?, widget)` — `src/tree.zig:24-29`. Task 1 adds a `meta.put` here capturing type/testID/text/parent. |
| `tree.apply` append arm | sets parent via `backend.appendChild`; the parent id is `op.parent.?` — capture it into `meta[child].parent` in the append/insertBefore arms — `src/tree.zig:30-33,40-44`. |
| `tree.apply` setText/update | `setText`/`update` arms must also refresh `meta[id].text`/`test_id` — `src/tree.zig:34-39`. |
| `tree.apply` remove | frees `nodes` entry — Task 1 also frees the `meta` entry's owned strings — `src/tree.zig:45-49`. |
| Button click emit connect | `gtk.Button.signals.clicked.connect(...)` exists (`src/gtk_backend.zig:73`); the signal struct exposes **only `.connect`, no `.emit`** (verified `vendor/.../gtk4.zig:4471`). Emit path is `gobject.signalEmitByName` or `gtk.Widget.activate` — see symbol table. |
| `the_window` | single `?*gtk.Window` in the backend (`src/gtk_backend.zig:8`); the screenshot/geometry root. Expose a getter `pub fn getWindow() ?*gtk.Window`. |
| props type | `op.props: ?std.json.Value` (`src/protocol.zig:42`); `propStr(props,"testID")` (existing `propStr` helper, `src/gtk_backend.zig:20`) extracts the string. |
| JSX intrinsics live in package | intrinsic prop types are in `packages/react/src/jsx-runtime.ts`'s own `JSX.IntrinsicElements` (jsxImportSource=@nativedesktop/react), **NOT** a `jsx.d.ts` — Task 7 adds `testID?: string` to all four there. |
| build.zig imports | single `gtk_imports` array (glib/gobject/gio/gtk) reused by exe+tests (`build.zig:16`). Task 3 adds `gsk`/`gdk`/`graphene` modules (`gobject.module("gsk4")` etc.) to this array. |
| CI | `.github/workflows/ci.yml` linear steps ending at `headless m3` (`.github/workflows/ci.yml:25`). Task 9 appends `headless m4`. |

### Verified-symbol table (re-verify inside the devshell before pasting)

All confirmed present this session in `vendor/gobject-bindings/`. Line numbers are from this session; re-run the `rg` if they drift — the **symbol name** is the contract.

| Need | Symbol (Zig binding) | C symbol | Verify command → expected |
|---|---|---|---|
| Window's live GSK renderer | `gtk.Native.getRenderer(native) ?*gsk.Renderer` | `gtk_native_get_renderer` | `rg -n "gtk_native_get_renderer\b" vendor/gobject-bindings/src/gtk4/gtk4.zig` → `63210` |
| Widget → its GtkNative | `gtk.Widget.getNative(widget) ?*gtk.Native` | `gtk_widget_get_native` | `rg -n "gtk_widget_get_native\b" .../gtk4/gtk4.zig` → `57357` |
| New snapshot recorder | `gtk.Snapshot.new() *gtk.Snapshot` | `gtk_snapshot_new` | `rg -n "gtk_snapshot_new\b" .../gtk4/gtk4.zig` → `43627` |
| Snapshot a child widget into a recorder | `gtk.Widget.snapshotChild(parent, child, snapshot)` | `gtk_widget_snapshot_child` | `rg -n "gtk_widget_snapshot_child\b" .../gtk4/gtk4.zig` → `58413` |
| Finish recorder → render node (consumes) | `gtk.Snapshot.freeToNode(snapshot) ?*gsk.RenderNode` | `gtk_snapshot_free_to_node` | `rg -n "gtk_snapshot_free_to_node\b" .../gtk4/gtk4.zig` → `43741` |
| Widget paintable (fallback snapshot path) | `gtk.WidgetPaintable.new(?widget) *gtk.WidgetPaintable` (implements `gdk.Paintable`) | `gtk_widget_paintable_new` | `rg -n "gtk_widget_paintable_new\b" .../gtk4/gtk4.zig` → `58514` |
| Paintable snapshot (fallback) | `gdk.Paintable.snapshot(paintable, gdk_snapshot, w, h)` | `gdk_paintable_snapshot` | `rg -n "gdk_paintable_snapshot\b" .../gdk4/gdk4.zig` → `6426` |
| Render node → texture | `gsk.Renderer.renderTexture(renderer, node, ?viewport) *gdk.Texture` | `gsk_renderer_render_texture` | `rg -n "gsk_renderer_render_texture\b" .../gsk4/gsk4.zig` → `1785` |
| Standalone cairo renderer (fallback if native has none) | `gsk.CairoRenderer.new() *gsk.CairoRenderer` + `gsk.Renderer.realize(r, ?surface, ?err)` | `gsk_cairo_renderer_new` / `gsk_renderer_realize` | `rg -n "gsk_cairo_renderer_new\b\|gsk_renderer_realize\b" .../gsk4/gsk4.zig` → `254`,`1752` |
| Save texture to PNG file | `gdk.Texture.saveToPng(texture, [*:0]path) c_int` | `gdk_texture_save_to_png` | `rg -n "gdk_texture_save_to_png\b" .../gdk4/gdk4.zig` → `5777` |
| Free a render node | `gsk.RenderNode.unref(node)` | `gsk_render_node_unref` | `rg -n "gsk_render_node_unref\b" .../gsk4/gsk4.zig` → `1663` |
| Compute bounds to a target widget | `gtk.Widget.computeBounds(widget, target, *graphene.Rect) c_int` | `gtk_widget_compute_bounds` | `rg -n "gtk_widget_compute_bounds\b" .../gtk4/gtk4.zig` → `56951` |
| Widget root (for bounds target) | `gtk.Widget.getRoot(widget) ?*gtk.Root` | `gtk_widget_get_root` | `rg -n "gtk_widget_get_root\b" .../gtk4/gtk4.zig` → `57455` |
| Widget width/height (logical px) | `gtk.Widget.getWidth`/`getHeight(widget) c_int` | `gtk_widget_get_width`/`_get_height` | `rg -n "gtk_widget_get_width\b" .../gtk4/gtk4.zig` → `57610` |
| Visible / mapped | `gtk.Widget.getVisible`/`getMapped(widget) c_int` | `gtk_widget_get_visible`/`_get_mapped` | `rg -n "gtk_widget_get_mapped\b" .../gtk4/gtk4.zig` → `57326` |
| Emit `clicked` (semantic click) | `gobject.signalEmitByName(@ptrCast(button), "clicked")` **or** `gtk.Widget.activate(widget) c_int` | `g_signal_emit_by_name` / `gtk_widget_activate` | `rg -n "g_signal_emit_by_name\b" .../gobject2/gobject2.zig` → `5712`; `rg -n "gtk_widget_activate\b" .../gtk4/gtk4.zig` → `56808` |
| graphene Rect layout | `graphene.Rect{ f_origin: Point, f_size: Size }` (`f_origin.f_x/f_y`, `f_size.f_width/f_height`) | — | `sed -n '1438,1442p' .../graphene1/graphene1.zig` |
| UI-thread marshal | `glib.MainContext.default().invokeFull(prio, cb, data, null)` | `g_main_context_invoke_full` | `rg -n "g_main_context_invoke_full\b" .../glib2/glib2.zig` → `5287` |

std concurrency + net symbols (Zig 0.16 std, verified this session):

| Need | Symbol | Verify command |
|---|---|---|
| Block automation thread on UI response | `std.Io.Condition` (`.init`, `wait(io, *Mutex)`, `signal(io)`) paired with `std.Io.Mutex` | `rg -n "pub const Condition" $(zig env \| rg -o '/nix/[^"]+/lib/zig')/std/Io.zig` → present (~`1653`) |
| One-shot alternative | `io.futexWait(u32, *ptr, expect)` / `io.futexWaitUncancelable` | same `Io.zig` |
| Accept one client | `std.Io.net.Server.accept(server, io) !Stream` | `rg -n "pub fn accept\b" $(…)/std/Io/net.zig` |

**`click` emit — chosen path.** Use `gtk.Widget.activate(widget)` for the Button (it emits `clicked` for a `GtkButton`) as the primary, with `gobject.signalEmitByName(@ptrCast(@alignCast(button)), "clicked")` as the fallback if `activate` proves to no-op headless. Verify at implement-time (Task 5 Step 2) by asserting the demo's `ND_CLICKED`-equivalent event/state change fires; record which worked. Both are verified-present; the choice is which reliably drives the counter under weston.

**`screenshot` — chosen path.** Primary: `gtk.Widget.getNative(window_widget)` → `gtk.Native.getRenderer(native)` (the live realized renderer) → build a `gtk.Snapshot.new()`, `snapshotChild(window, target, snapshot)` (target defaults to the window widget), `freeToNode` → `renderer.renderTexture(node, null)` → `texture.saveToPng(path)` → read back `texture` width/height via `gdk.Texture` getters (or the target widget's `getWidth`/`getHeight`). Fallback (if `getRenderer` returns null under headless): a standalone `gsk.CairoRenderer.new()` realized against the window's surface (`gtk.Native.getSurface`). Task 4 Step 2 verifies which the weston/cairo backend provides and records it. **Coordinate-space contract** (research gotcha): all geometry in `getTree` is in **logical units** (GTK px, the same space GTK layout uses), origin top-left of the window's content, and the `getTree` result states `"coordinateSpace":"logical-window-topleft"` explicitly so a computer-use client knows screenshots (physical px, HiDPI-scaled) are NOT 1:1 with these numbers.

---

## RPC surface v1 (the contract — implement exactly)

All messages are JSON-RPC 2.0 inside the NDP outer frame. `id` is an integer echoed in the response.

### `getTree` → tree snapshot

Request: `{"jsonrpc":"2.0","id":N,"method":"getTree"}` (no params).

Result:
```json
{
  "coordinateSpace": "logical-window-topleft",
  "root": {
    "ref": 16777217,
    "type": "Window",
    "testID": null,
    "text": null,
    "visible": true,
    "geometry": { "x": 0, "y": 0, "w": 480, "h": 320 },
    "children": [
      { "ref": 16777218, "type": "Box", "testID": null, "text": null, "visible": true,
        "geometry": {"x":0,"y":0,"w":480,"h":320}, "children": [
          { "ref": 16777219, "type": "Label", "testID": "clicks-label", "text": "Clicks: 0",
            "visible": true, "geometry": {"x":0,"y":0,"w":480,"h":24}, "children": [] },
          { "ref": 16777220, "type": "Button", "testID": "increment-button", "text": "Increment",
            "visible": true, "geometry": {"x":0,"y":24,"w":480,"h":40}, "children": [] }
        ] }
    ]
  }
}
```
- `ref` = the stable node id (the generation-tagged u32 from `ids.ts`; a plain JSON number).
- `type` = widget-type string (`"Window"|"Box"|"Label"|"Button"`).
- `testID` = the developer `testID` prop, or `null`.
- `text` = the Label text or Button label if any, else `null`.
- `visible` = `getVisible(widget) != 0`.
- `geometry` = `{x,y,w,h}` logical units, computed via `computeBounds(widget, window_widget, &rect)` (x/y relative to the window's top-left); if `computeBounds` returns 0 (not yet allocated), emit `{x:0,y:0,w:getWidth,h:getHeight}` and set `"geometry":null` only if the widget is unmapped.
- Served by marshaling to the UI thread (**must answer even if the JS child is stalled** — D11 SLO).

### `screenshot` → PNG to a path

Request: `{"jsonrpc":"2.0","id":N,"method":"screenshot","params":{"path":"/abs/out.png","window":REF?}}`.
- `path` (required): absolute file path to write the PNG.
- `window` (optional): a window ref; defaults to the single `the_window`. (Multi-window is M5+; if `window` names a non-window ref, error `-32602`.)

Result: `{"path":"/abs/out.png","width":480,"height":320}`. In-process GTK→GSK render (no OS capture). D11: must answer while the child is stalled.

### `click` → semantic dispatch with actionability

Request: `{"jsonrpc":"2.0","id":N,"method":"click","params":{"ref":REF}}`.
- Actionability hit-test first (exists ∧ visible ∧ mapped ∧ bounds contain center). On failure: error `{"code":-32001,"message":"not actionable","data":{"ref":REF,"reason":"invisible|unmapped|unknown|offscreen"}}`.
- On pass: emit `clicked` (via `activate`) on the UI thread.

Result: `{"ref":REF,"dispatched":true}`.

### `setValue` / `type` / `scroll` → not implemented

Request e.g. `{"jsonrpc":"2.0","id":N,"method":"setValue","params":{"ref":REF,"value":"…"}}`.
Response: error `{"code":-32601,"message":"not implemented until M5 widgets","data":{"method":"setValue"}}`. Signatures fixed now: `setValue{ref,value:string}`, `type{ref,text:string}`, `scroll{ref,dx:number,dy:number}`.

### `waitFor` → poll the tree

Request: `{"jsonrpc":"2.0","id":N,"method":"waitFor","params":{"condition":{"textContains":"Clicks: 3"},"timeoutMs":2000}}`.
- `condition` is one of `{"textContains": string}` (any node's `text` contains the substring) or `{"refVisible": REF}` (that ref exists and is visible).
- Polls the tree on the UI thread at ~50 ms until satisfied or `timeoutMs` elapses.

Result on success: `{"matched":true}`. On timeout: error `{"code":-32002,"message":"waitFor timeout","data":{"timeoutMs":T}}`.

### Errors

Unknown method → `-32601`. Bad params → `-32602`. Parse error → `-32700`. Internal → `-32603`. Not-actionable → `-32001`. waitFor timeout → `-32002`.

---

### Task 1: Host-side node-metadata registry + `testID` storage (TDD)

**Files:**
- Modify: `src/tree.zig` (add `NodeMeta` + `meta` map; populate/free in `apply`)
- Modify: `src/gtk_backend.zig` (strip `testID` from props before widget creation; add `pub fn getWindow`)

**Interfaces:**
- Consumes: `protocol.Op`, existing `propStr`.
- Produces:
  - `tree.NodeMeta = struct { widget_type: []u8, test_id: ?[]u8, text: ?[]u8, parent: u32 }` (owned, allocator-backed copies).
  - `Tree.meta: std.AutoHashMapUnmanaged(u32, NodeMeta)` populated on `create` (type + testID + initial text + parent=0), updated on `append`/`insertBefore` (parent), `setText`/`update` (text/testID), freed on `remove`.
  - `Tree.metaGet(id) ?*NodeMeta`, `Tree.rootId() ?u32` (the sole Window node id).
  - `gtk_backend.getWindow() ?*gtk.Window`.

- [ ] **Step 1: Add a `tree.zig` decode/store unit test FIRST**

Add to `src/tree.zig` (a `test` block; the tree test currently isn't a build target — wire it: append `tree` as a root to the `test_step` in `build.zig`, mirroring `protocol_tests`, OR make the test import-only by putting it in `protocol.zig`. Simplest: add a standalone `tree_tests` target in `build.zig` needing the gtk imports). The test constructs a `Tree` (no real GTK app needed for the meta map — test `NodeMeta` alloc/free in isolation): insert two metas, assert `metaGet` returns the stored type/testID/text, `remove` frees without leak (use `std.testing.allocator` — it fails the test on leak).

```zig
test "node meta stores type/testID/text and frees on remove" {
    const gpa = std.testing.allocator;
    var t = Tree.initBare(gpa); // a ctor that skips the *gtk.Application for pure-meta tests
    defer t.deinitMeta();
    try t.putMeta(1, "Button", "increment-button", "Increment", 0);
    const m = t.metaGet(1).?;
    try std.testing.expectEqualStrings("Button", m.widget_type);
    try std.testing.expectEqualStrings("increment-button", m.test_id.?);
    try std.testing.expectEqualStrings("Increment", m.text.?);
    t.removeMeta(1);
    try std.testing.expect(t.metaGet(1) == null);
}
```
Run: `nix develop -c zig build test` — fails (symbols absent). This defines the meta API surface.

- [ ] **Step 2: Implement `NodeMeta` + the meta map in `src/tree.zig`**

Add the struct and map field, plus helpers. `putMeta` dupes all strings via `self.gpa`; `removeMeta` frees them. In `apply`'s `create` arm (after `nodes.put`), extract the testID and initial text from `op.props` (`propStr(op.props, "testID")`, and for Label/Button the `text`/`label` prop) and call `putMeta(op.id.?, op.widget.?, test_id, text, 0)`. In the `append`/`insertBefore` arms set `meta[child].parent = parent_id` (add a `setMetaParent`). In `setText` update `meta[id].text`. In `update`, if `props.testID` changed, refresh `meta[id].test_id`. In `remove`, call `removeMeta(op.id.?)`. Add `rootId()` returning the id whose meta `widget_type == "Window"` (there is one). Free all meta in a `deinitMeta`.

Verify `propStr` handles the testID lookup: it returns `?[]const u8` for object string fields (`src/gtk_backend.zig:20`) — import/re-expose it or duplicate the tiny helper in `tree.zig` (it currently lives in `gtk_backend`). Prefer moving `propStr`/`propInt` to a shared spot or re-declaring a local `propStr` in `tree.zig` — smallest diff: a local copy.

- [ ] **Step 3: Strip `testID` before widget creation + add `getWindow` in `src/gtk_backend.zig`**

`testID` must not reach `createWidget`/`applyProps` as a widget property. Since `createWidget` only reads known keys (`title`/`orientation`/`text`/`label`/…) it already ignores unknown keys, so no active stripping is strictly required — but add an explicit guard comment and ensure no future `applyProps` generic-copy path forwards it. Add:
```zig
pub fn getWindow() ?*gtk.Window {
    return the_window;
}
```

- [ ] **Step 4: Build + test**

Run: `nix develop -c bash -c 'zig build test && zig build'`
Expected: the new meta test passes; host builds. The M2/M3 headless paths are unaffected (meta is additive). Fix the first error only.

- [ ] **Step 5: Regression-check M3 still green**

Run: `nix develop -c bash -c './scripts/headless-m3.sh && ./scripts/kill9-test.sh'`
Expected: `headless m3: OK`, `kill9: OK`. Meta population must not perturb the commit path.

- [ ] **Step 6: Commit**
```bash
git add src/tree.zig src/gtk_backend.zig build.zig
git commit -m "feat: host node-metadata registry with testID/type/text/parent"
```

### Task 2: Automation server thread + framed JSON-RPC dispatch + UI-thread marshal primitive

**Files:**
- Create: `src/automation.zig` (the server: listener thread, accept-one-client loop, JSON-RPC parse/dispatch, the marshal-and-block helper)
- Modify: `src/main.zig` (start the automation server when `NATIVE_AUTOMATION=1`)
- Modify: `build.zig` (add `gsk`/`gdk`/`graphene` imports to `gtk_imports`; ensure `automation.zig` compiles via `main.zig` import)

**Interfaces:**
- Consumes: `Tree` (+ new meta API), `gtk_backend.getWindow`, `protocol.encodeFrame`, `std.Io.net`, `glib.MainContext`.
- Produces:
  - `automation.Server = struct { start(gpa, io, tree, sock_path) !*Server }` spawning a listener `std.Thread`.
  - A `UiCall` mechanism: a request struct `{ done: std.Io.Condition, mutex: std.Io.Mutex, io: std.Io, /* per-op in/out fields */ }`, an `invokeFull` onto `glib.MainContext.default()` whose callback fills the out fields and `signal`s `done`; the automation thread `wait`s on `done`.
  - `dispatch(req_json) -> response_json` routing on `method`.

- [ ] **Step 1: Scaffold `src/automation.zig` — listener thread + accept-one loop (no handlers yet)**

Mirror `runtime.zig`'s socket setup. The server owns its own `std.Io.Threaded`? No — reuse the runtime's `io`. Simplest: `start` receives the `std.Io` from the caller (main creates one `Threaded` for the runtime already; pass its `io`). Build the socket path `nd-automation-<pid>.sock`, `deleteFileAbsolute` any stale, `UnixAddress.init` → `listen`. Print `ND_AUTOMATION_LISTENING path=<sock>`. Spawn `std.Thread.spawn(.{}, listenLoop, .{self})`.

`listenLoop`: `while (true) { const stream = server.accept(io) catch break; print ND_AUTOMATION_CONNECTED; serveClient(stream); print ND_AUTOMATION_DISCONNECTED; }` — serving one client fully before accepting the next.

`serveClient`: read frames with a length-prefix reader (copy the `readFrame` shape from `runtime.zig:164` as a local `readFrame`), parse JSON-RPC, `dispatch`, `encodeFrame` the response, write it back. On read error, return (client gone), loop re-accepts.

For now `dispatch` returns a stub `{"jsonrpc":"2.0","id":id,"result":{"ok":true}}` for any method — real handlers land in Tasks 3–6.

- [ ] **Step 2: Implement the marshal-and-block UI-call primitive**

The core D11 mechanism. Define:
```zig
const UiJob = struct {
    tree: *Tree,
    // input
    kind: enum { get_tree, screenshot, click, wait_poll },
    // params (tagged by kind)
    ref: u32 = 0,
    path: ?[:0]const u8 = null,
    // output (filled on UI thread)
    result_json: ?[]u8 = null,   // owned by gpa; the automation thread frees
    err_code: i32 = 0,
    err_msg: ?[]const u8 = null,
    gpa: std.mem.Allocator,
    io: std.Io,
    mutex: std.Io.Mutex = .init,
    done: std.Io.Condition = .init,
    finished: bool = false,
};

fn runOnUi(job: *UiJob) void {
    _ = glib.MainContext.default().invokeFull(glib.PRIORITY_DEFAULT, &uiCallback, job, null);
    job.mutex.lockUncancelable(job.io);
    defer job.mutex.unlock(job.io);
    while (!job.finished) job.done.waitUncancelable(job.io, &job.mutex);
}

fn uiCallback(data: ?*anyopaque) callconv(.c) c_int {
    const job: *UiJob = @ptrCast(@alignCast(data.?));
    handleOnUi(job); // fills result_json / err_* — Tasks 3-6
    job.mutex.lockUncancelable(job.io);
    job.finished = true;
    job.done.signal(job.io);
    job.mutex.unlock(job.io);
    return 0; // G_SOURCE_REMOVE
}
```
`handleOnUi` is a switch on `job.kind`; Tasks 3–6 fill each arm. Verify `std.Io.Condition` has `signal(io)` and `waitUncancelable(io, *Mutex)` (verify: `rg -n "pub fn signal\b|pub fn waitUncancelable\b" $(zig env | rg -o '/nix/[^"]+/lib/zig')/std/Io.zig`). If `signal` is named `notify`/`wake`, adjust.

**Why this answers under a stalled child (D11):** `uiCallback` runs on the GLib main loop, which keeps iterating independent of the Bun child (the child only feeds `CommitBatch`es via a separate reader thread). Reading `tree.meta` + GTK geometry needs no child cooperation. The automation thread blocks only on the main loop, never on the child.

- [ ] **Step 3: Start the server from `main.zig` behind `NATIVE_AUTOMATION=1`**

In `onActivate` (the M2 branch, after `Runtime.start`), add:
```zig
if (global_environ_map.?.get("NATIVE_AUTOMATION")) |v| {
    if (std.mem.eql(u8, v, "1")) {
        _ = automation.Server.start(gpa, /* io from runtime */, &tree) catch |err|
            std.debug.print("ND_AUTOMATION_ERROR {any}\n", .{err});
    }
}
```
The `io` must be the runtime's `std.Io` (the automation thread shares it). Either expose `Runtime.io` (add a `pub` getter) or have `automation.Server.start` take the `*Runtime`. Prefer passing `rt.io` — add `pub fn getIo(self: *Runtime) std.Io { return self.io; }` to `runtime.zig` (one-line, non-breaking).

- [ ] **Step 4: Wire build.zig imports and compile**

Add to `gtk_imports` in `build.zig`:
```zig
.{ .name = "gsk", .module = gobject.module("gsk4") },
.{ .name = "gdk", .module = gobject.module("gdk4") },
.{ .name = "graphene", .module = gobject.module("graphene1") },
```
Verify the module names exist: `rg -n "gsk4|gdk4|graphene1" vendor/gobject-bindings/build.zig` (the vendored build exposes these modules — the `src/gsk4`,`src/gdk4`,`src/graphene1` dirs confirm). `automation.zig` is imported by `main.zig` so it compiles in the exe + test modules automatically.

Run: `nix develop -c bash -c 'zig build'`
Expected: builds. Fix the first error only (likely a module-name typo or a `std.Io.Condition` method name).

- [ ] **Step 5: Smoke the listener manually**

Run:
```bash
nix develop -c bash -c 'GSK_RENDERER=cairo NATIVE_AUTOMATION=1 timeout 2 ./zig-out/bin/nd-hello --smoke 2>&1 | grep -E "ND_AUTOMATION_LISTENING"'
```
(Use `--smoke` so no Bun child is needed to prove the listener opens; the socket path prints regardless.) Expected: one `ND_AUTOMATION_LISTENING path=.../nd-automation-<pid>.sock` line. If `--smoke` bypasses `onActivate`'s M2 branch (it does — `--smoke` returns early), instead run without `--smoke` under weston; simplest is to fold the automation start into a spot reached by both branches, or accept testing it in Task 9's headless harness. Record which. Fix the first failure only.

- [ ] **Step 6: Commit**
```bash
git add src/automation.zig src/main.zig src/runtime.zig build.zig
git commit -m "feat: automation server thread with framed json-rpc and ui-thread marshal"
```

### Task 3: `getTree` handler

**Files:**
- Modify: `src/automation.zig` (the `get_tree` arm of `handleOnUi` + `dispatch` routing + JSON serialization)

**Interfaces:**
- Consumes: `Tree.meta`, `Tree.rootId`, `gtk_backend.getWindow`, `gtk.Widget.computeBounds`, `graphene.Rect`.
- Produces: a `getTree` result JSON (the shape in the RPC surface section) built on the UI thread into `job.result_json`.

- [ ] **Step 1: Build the nested snapshot on the UI thread**

In `handleOnUi` `.get_tree`: starting from `tree.rootId()`, recurse the `meta` map by `parent` to build a nested tree. For each node: read `meta.widget_type`/`test_id`/`text`; get the live widget via `tree.get(id)`; `visible = gtk.Widget.getVisible(widget) != 0`; compute geometry via `computeBounds(widget, window_widget, &rect)` where `window_widget = getWindow().?.as(gtk.Widget)` and rect fields are `rect.f_origin.f_x`, `rect.f_origin.f_y`, `rect.f_size.f_width`, `rect.f_size.f_height` (cast f32→ integer logical px). Serialize with `std.json.Stringify.valueAlloc` over a Zig struct tree, or build the JSON string directly. Prefer a `TreeNode` struct with an owned `children: []TreeNode` slice and `std.json.Stringify` — matches `protocol.encodeFrame`'s approach. Set top-level `coordinateSpace = "logical-window-topleft"`.

Children order: reconstruct from `meta.parent` links. Since GTK keeps live child order, an alternative is to walk `gtk.Widget.getFirstChild`/`getNextSibling` from the window and map each live widget back to its id via a reverse lookup — this gives **true visual order** and is more robust than insertion order. Verified symbols: `gtk_widget_get_first_child` (57195), `gtk_widget_get_next_sibling` (57363). Build a `widget→id` reverse map from `tree.nodes` once per call. Prefer the live-child-walk for correct ordering; fall back to `meta.parent` grouping only if the reverse map is awkward. Record the choice.

- [ ] **Step 2: Route `getTree` in `dispatch` and serialize the response**

`dispatch` parses `{jsonrpc,id,method,params}`, on `method=="getTree"` constructs a `UiJob{.kind=.get_tree}`, `runOnUi(&job)`, then wraps `job.result_json` as `{"jsonrpc":"2.0","id":id,"result": <result_json>}`. Since `result_json` is already-serialized JSON, splice it (build the envelope with the raw result bytes inserted) rather than double-encoding — a small `std.fmt`/manual concat, or make `result` a `std.json.Value` parsed back. Simplest: have `handleOnUi` fill a `std.json.Value`-serializable struct and let `dispatch` do one `Stringify` of the whole envelope. Refactor `UiJob.result_json` to instead hold a `*TreeNode` (owned) and serialize the envelope once in `dispatch`. Pick one and keep it consistent across Tasks 3–6.

- [ ] **Step 3: Test via a raw socket poke (interim, no MCP)**

Write a throwaway one-liner (do not commit) or use Task 9's driver early: under weston with `NATIVE_AUTOMATION=1` and the counter running, connect to the automation socket, send a framed `getTree`, print the reply. Assert the reply contains `"increment-button"` and `"Clicks: 0"`. This is folded into Task 9's `m4-drive.ts`; here just prove the wire once with:
```bash
# after Task 9's script exists, or an ad-hoc bun snippet — verification only, no commit
```
If tested ad-hoc, delete the snippet. Expected: a JSON tree with the window→box→labels/button structure and correct `testID`s.

- [ ] **Step 4: Commit**
```bash
git add src/automation.zig
git commit -m "feat: getTree rpc returns nested snapshot with refs, testid, geometry"
```

### Task 4: `screenshot` handler

**Files:**
- Modify: `src/automation.zig` (the `screenshot` arm + routing)

**Interfaces:**
- Consumes: `gtk_backend.getWindow`, `gtk.Native`, `gsk.Renderer`, `gtk.Snapshot`, `gdk.Texture`.
- Produces: a PNG at the caller's `path`; result `{path,width,height}`.

- [ ] **Step 1: Verify the render path under headless before coding it**

The exact renderer availability under `GSK_RENDERER=cairo` + weston must be confirmed. Add a temporary debug print in the `screenshot` arm: whether `getNative(window)` and `getRenderer(native)` return non-null. Run under Task 9's harness (or ad-hoc). If `getRenderer` is non-null, use it; if null, realize a `gsk.CairoRenderer` against `getSurface(native)`.

- [ ] **Step 2: Implement the in-process render**

On the UI thread:
```zig
const win_widget = getWindow().?.as(gtk.Widget);
const native = gtk.Widget.getNative(win_widget) orelse { set err -32603 "no native"; return; };
const renderer = gtk.Native.getRenderer(native) orelse { /* fallback cairo renderer */ };
const snapshot = gtk.Snapshot.new();
gtk.Widget.snapshotChild(win_widget, target_widget, snapshot); // target defaults to win_widget's child; see note
const node = gtk.Snapshot.freeToNode(snapshot) orelse { set err "empty snapshot"; return; };
defer gsk.RenderNode.unref(node);
const texture = gsk.Renderer.renderTexture(renderer, node, null);
defer texture-unref; // gdk.Texture is a GObject; unref via gobject
const ok = gdk.Texture.saveToPng(texture, path_z); // c_int nonzero = success
```
**Note on `snapshotChild`:** `gtk_widget_snapshot_child(parent, child, snapshot)` snapshots `child` in `parent`'s coordinate space — so to capture the whole window, snapshot the window's child (the box) via the window as parent, OR use the `WidgetPaintable` route (`gtk.WidgetPaintable.new(win_widget)` → cast `.as(gdk.Paintable)` → `gdk.Paintable.snapshot(paintable, gdk_snapshot, w, h)`) which snapshots the whole widget cleanly. **Prefer the WidgetPaintable route** for a full-window capture (it takes the widget directly, no parent/child juggling): `gtk_widget_paintable_new(win_widget)` gives a paintable; `gdk.Paintable.snapshot` needs a `gdk.Snapshot` — a `gtk.Snapshot` **is** a `gdk.Snapshot` subtype (cast `.as(gdk.Snapshot)`). Verify the cast compiles (`gtk.Snapshot` parent chain includes `gdk.Snapshot`); if not, use the `snapshotChild` route on the window's child. Record which route rendered a non-empty PNG under weston.

Set `job.result_json` = `{path,width,height}` using the window's `getWidth`/`getHeight`.

- [ ] **Step 3: Route `screenshot` in `dispatch`; parse `params.path`**

Require `params.path` (error `-32602` if missing). Dupe it null-terminated (`gpa.dupeZ`) for `saveToPng`. Optional `params.window` ignored in v1 beyond validating it's the window ref (else `-32602`).

- [ ] **Step 4: Verify a real PNG is written**

Under Task 9's harness the driver asserts non-empty PNG. Here, ad-hoc: run the counter with `NATIVE_AUTOMATION=1`, screenshot to `/tmp/m4.png`, then `file /tmp/m4.png` → `PNG image data`, and `[ -s /tmp/m4.png ]`. Expected: a valid non-empty PNG. Fix the first failure (most likely the renderer-null fallback or an empty snapshot on an unmapped window — ensure the window is mapped first, which under weston it is once presented).

- [ ] **Step 5: Commit**
```bash
git add src/automation.zig
git commit -m "feat: screenshot rpc renders window in-process to a png file"
```

### Task 5: `click` handler with actionability hit-test + semantic dispatch

**Files:**
- Modify: `src/automation.zig` (the `click` arm + routing)

**Interfaces:**
- Consumes: `tree.get(ref)`, `getVisible`/`getMapped`/`computeBounds`, `gtk.Widget.activate` (fallback `gobject.signalEmitByName`).
- Produces: actionability-checked semantic click; result `{ref,dispatched:true}` or error `-32001`.

- [ ] **Step 1: Implement the actionability hit-test**

On the UI thread, for `job.ref`:
1. `const widget = tree.get(ref) orelse { err -32001 reason "unknown"; return; }`.
2. `if (gtk.Widget.getVisible(widget) == 0) { err reason "invisible"; return; }`.
3. `if (gtk.Widget.getMapped(widget) == 0) { err reason "unmapped"; return; }`.
4. `computeBounds(widget, getWindow().?.as(gtk.Widget), &rect)` — if it returns 0, `err reason "offscreen"`. Compute the center `(cx,cy) = (origin.x + size.w/2, origin.y + size.h/2)` and confirm the rect contains it (trivially true for its own bounds — the meaningful check is that bounds are non-degenerate: `size.w > 0 and size.h > 0`; a zero-size or unallocated widget fails as `"offscreen"`). This encodes the research gotcha: never dispatch to a widget a user couldn't reach. (Full z-order/overlap hit-testing against sibling coverage is deferred; M4's check is existence+visible+mapped+non-degenerate-bounds, which is the achievable subset for the current 4-widget set.)

- [ ] **Step 2: Semantic dispatch — emit `clicked`**

On pass: `_ = gtk.Widget.activate(widget)`. Verify under weston it drives the counter's `onClick` (the child receives the `clicked` NDP event → `setState`). If `activate` no-ops for the button headless, use `gobject.signalEmitByName(@ptrCast(@alignCast(widget)), "clicked")`. Test in Task 9 by asserting `Clicks: 3` after three clicks; here record which emit worked. Set `job.result_json = {ref,dispatched:true}`.

- [ ] **Step 3: Route `click`; parse `params.ref`**

Require integer `params.ref` (else `-32602`). Build `UiJob{.kind=.click,.ref=ref}`.

- [ ] **Step 4: Verify (ad-hoc or fold into Task 9)**

Click the increment button once via the socket; assert the child logs a `clicked` event / the next `getTree` shows `Clicks: 1`. Expected: the counter increments. Fix the first failure.

- [ ] **Step 5: Commit**
```bash
git add src/automation.zig
git commit -m "feat: click rpc with actionability hit-test and semantic clicked dispatch"
```

### Task 6: `waitFor` + not-implemented stubs (`setValue`/`type`/`scroll`)

**Files:**
- Modify: `src/automation.zig` (`wait_poll` arm + routing; the three stub routes)

**Interfaces:**
- Consumes: the `get_tree` traversal (reused for condition checks).
- Produces: `waitFor` polling; `setValue`/`type`/`scroll` → `-32601` "not implemented until M5 widgets".

- [ ] **Step 1: Implement `waitFor` polling on the automation thread**

`waitFor` polls: the automation thread loops, each iteration `runOnUi` a `.wait_poll` job that evaluates the condition against the live tree (reuse the traversal from Task 3: for `textContains`, any node whose `text` contains the substring; for `refVisible`, `tree.get(ref)` exists ∧ visible), returning a bool. Between polls the automation thread sleeps ~50 ms (`io.sleep`? verify — else a short `std.Io` timer; if none, a `std.Thread.sleep(50 * std.time.ns_per_ms)` on the automation thread is acceptable since it's not the UI thread). Loop until true (→ `{matched:true}`) or `timeoutMs` elapsed (→ error `-32002`). Track elapsed with `std.time.milliTimestamp`.

The condition evaluation runs on the UI thread (each poll is a marshaled read); the sleep and the deadline check run on the automation thread. This keeps all tree access UI-thread-only.

- [ ] **Step 2: Implement the three not-implemented stubs**

In `dispatch`, `setValue`/`type`/`scroll` short-circuit **without** a `UiJob` (no widget touch needed) → return error `{"code":-32601,"message":"not implemented until M5 widgets","data":{"method":<m>}}`. Validate they at least parse a `ref` (so the signature is exercised), but do not act.

- [ ] **Step 3: Route + build/test**

Run: `nix develop -c bash -c 'zig build test && zig build'`
Expected: builds and existing tests pass. Fix the first error.

- [ ] **Step 4: Commit**
```bash
git add src/automation.zig
git commit -m "feat: waitFor polling and notImplemented stubs for setValue/type/scroll"
```

> **Zig automation-server track (Tasks 1–6) is complete and buildable here.** The JS + MCP track (Tasks 7–8) can proceed in parallel from the start (it shares no files).

---

### Task 7: `testID` intrinsic prop through `packages/react` + counter demo

**Files:**
- Modify: `packages/react/src/jsx-runtime.ts` (add `testID?: string` to all four intrinsics)
- Modify: `packages/react/src/host-config.ts` (do NOT strip `testID` from props — it must flow to the wire; ensure the generic prop diff/copy forwards it)
- Modify: `examples/counter/main.tsx` (add `testID` to the button + clicks label)

**Interfaces:**
- Consumes: the existing `create`/`update` op `props` path (`emitCreateIfNew` copies `inst.props` minus `children`/`onClick`).
- Produces: `props.testID` present in the `create` op JSON for tagged widgets; recognized by Task 1's host storage.

- [ ] **Step 1: Add `testID` to the JSX intrinsics**

In `packages/react/src/jsx-runtime.ts`'s `JSX.IntrinsicElements`, add `testID?: string;` to each of `window`, `box`, `label`, `button`.

- [ ] **Step 2: Confirm `testID` reaches the wire**

`emitCreateIfNew` (`host-config.ts:43`) copies `inst.props`, deletes `children` and `onClick`, and pushes the rest as the `create` op's `props`. `testID` is neither deleted nor special-cased → it already flows. **Verify no change is needed**; do not add code. For `commitUpdate`, the generic changed-prop loop (`host-config.ts:125`) skips `children`/`onClick` and the label `text` — `testID` would flow as an `update` prop if it changed (it won't in the demo, but correct). No host-config code change; only the type addition in Step 1. (If tsc complains that `testID` is an unknown intrinsic prop, Step 1 fixed it.)

- [ ] **Step 3: Add `testID` to the counter demo (minimal diff)**

In `examples/counter/main.tsx`, change:
```tsx
<label testID="clicks-label" text={`Clicks: ${clicks}`} />
```
```tsx
<button testID="increment-button" label="Increment" onClick={onClick} />
```
Nothing else changes.

- [ ] **Step 4: Type-check**

Run: `nix develop -c bash -c 'cd packages/react && bunx tsc --noEmit && cd ../../examples/counter && bunx tsc --noEmit'`
Expected: clean. Fix the first error only.

- [ ] **Step 5: Commit**
```bash
git add packages/react/src/jsx-runtime.ts examples/counter/main.tsx
git commit -m "feat: testID intrinsic prop plumbed to props json and set on counter widgets"
```

### Task 8: `packages/mcp` — stdio MCP wrapper over the automation socket

**Files:**
- Create: `packages/mcp/package.json`
- Create: `packages/mcp/tsconfig.json`
- Create: `packages/mcp/src/socket.ts` (framed JSON-RPC client over the unix socket)
- Create: `packages/mcp/src/index.ts` (the MCP server: four tools)
- Create/commit: `bun.lock` update

**Interfaces:**
- Consumes: `@modelcontextprotocol/sdk`, `ND_AUTOMATION_SOCKET` env, the framed JSON-RPC wire (same framing as `runtime/ndp.ts`).
- Produces: a stdio MCP server exposing `nd_get_tree`, `nd_screenshot`, `nd_click`, `nd_wait_for`.

- [ ] **Step 1: Verify the SDK package name/API, then write `package.json`**

Run:
```bash
nix develop -c bash -c 'bun info @modelcontextprotocol/sdk version'
```
Expected: a `1.x` version (verified `1.29.0` this session). Pin it exactly (no caret).

`packages/mcp/package.json`:
```json
{
  "name": "@nativedesktop/mcp",
  "version": "0.0.0",
  "private": true,
  "type": "module",
  "bin": { "nd-mcp": "src/index.ts" },
  "dependencies": {
    "@modelcontextprotocol/sdk": "1.29.0"
  }
}
```
(Replace `1.29.0` with the exact `bun info` output.)

`packages/mcp/tsconfig.json`:
```json
{
  "extends": "../../tsconfig.base.json",
  "compilerOptions": { "types": ["bun"] },
  "include": ["src/**/*.ts"]
}
```

- [ ] **Step 2: Write the framed-socket client (`src/socket.ts`)**

Reuse the framing logic from `runtime/ndp.ts` (u32 LE length + JSON), but as a request/response JSON-RPC client with an id→resolver map:
```ts
export class AutomationClient {
  private socket!: import("bun").Socket;
  private inbox = new Uint8Array(0);
  private nextId = 1;
  private pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: Error) => void }>();

  static async connect(path = process.env.ND_AUTOMATION_SOCKET): Promise<AutomationClient> {
    if (!path) throw new Error("ND_AUTOMATION_SOCKET not set");
    const self = new AutomationClient();
    self.socket = await Bun.connect({ unix: path, socket: {
      data: (_s, chunk) => self.onData(chunk),
      close: () => { for (const p of self.pending.values()) p.reject(new Error("automation socket closed")); },
    }});
    return self;
  }

  private onData(chunk: Uint8Array): void {
    const merged = new Uint8Array(this.inbox.length + chunk.length);
    merged.set(this.inbox, 0); merged.set(chunk, this.inbox.length);
    this.inbox = merged;
    while (this.inbox.length >= 4) {
      const len = new DataView(this.inbox.buffer, this.inbox.byteOffset, 4).getUint32(0, true);
      if (this.inbox.length < 4 + len) break;
      const msg = JSON.parse(new TextDecoder().decode(this.inbox.subarray(4, 4 + len)));
      this.inbox = this.inbox.subarray(4 + len);
      const p = this.pending.get(msg.id);
      if (!p) continue;
      this.pending.delete(msg.id);
      if (msg.error) p.reject(new Error(`${msg.error.message} (${msg.error.code})`));
      else p.resolve(msg.result);
    }
  }

  call(method: string, params?: unknown): Promise<unknown> {
    const id = this.nextId++;
    const json = new TextEncoder().encode(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
    const frame = new Uint8Array(4 + json.length);
    new DataView(frame.buffer).setUint32(0, json.length, true);
    frame.set(json, 4);
    return new Promise((resolve, reject) => { this.pending.set(id, { resolve, reject }); this.socket.write(frame); });
  }
}
```

- [ ] **Step 3: Write the MCP server (`src/index.ts`)**

Verify the SDK import surface first: `bun --print "Object.keys(await import('@modelcontextprotocol/sdk/server/mcp.js'))"` and `.../server/stdio.js`. The 1.x API is `McpServer` from `@modelcontextprotocol/sdk/server/mcp.js` + `StdioServerTransport` from `@modelcontextprotocol/sdk/server/stdio.js`, with `server.registerTool(name, {description, inputSchema}, handler)` (or `server.tool(...)` in older 1.x — confirm at implement-time and use what the installed version exports). Each tool calls the socket client and returns the result as MCP content:
```ts
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import { AutomationClient } from "./socket.ts";

const client = await AutomationClient.connect();
const server = new McpServer({ name: "nativedesktop", version: "0.0.0" });

server.registerTool("nd_get_tree", { description: "Snapshot the app widget tree with stable refs, testIDs, text, and logical geometry.", inputSchema: {} },
  async () => ({ content: [{ type: "text", text: JSON.stringify(await client.call("getTree"), null, 2) }] }));

server.registerTool("nd_screenshot", { description: "Render the window in-process to a PNG at the given absolute path.", inputSchema: { path: z.string() } },
  async ({ path }) => ({ content: [{ type: "text", text: JSON.stringify(await client.call("screenshot", { path })) }] }));

server.registerTool("nd_click", { description: "Semantic click on a widget by ref (actionability-checked).", inputSchema: { ref: z.number() } },
  async ({ ref }) => ({ content: [{ type: "text", text: JSON.stringify(await client.call("click", { ref })) }] }));

server.registerTool("nd_wait_for", { description: "Wait until a tree condition holds or timeout.", inputSchema: { textContains: z.string().optional(), refVisible: z.number().optional(), timeoutMs: z.number().default(2000) } },
  async ({ textContains, refVisible, timeoutMs }) => {
    const condition = textContains !== undefined ? { textContains } : { refVisible };
    return { content: [{ type: "text", text: JSON.stringify(await client.call("waitFor", { condition, timeoutMs })) }] };
  });

await server.connect(new StdioServerTransport());
```
Adjust `registerTool`/`inputSchema` shape to the installed SDK version (some 1.x expect a raw Zod object, some a `{ inputSchema: z.object(...) }`). The exact shape comes from the `bun --print` surface dump — do not guess; use what it prints.

- [ ] **Step 4: Install + type-check + smoke the MCP handshake**

Run:
```bash
nix develop -c bash -c 'bun install'
nix develop -c bash -c 'cd packages/mcp && bunx tsc --noEmit'
```
Expected: `bun.lock` updated with the SDK; tsc clean (`z` from `zod` is a transitive dep of the SDK — if unresolved, add `zod` to `dependencies` at the exact version the SDK pins, found via `bun info @modelcontextprotocol/sdk dependencies`). Smoke that the server at least starts and lists tools without a live socket is hard (it connects on boot); defer the live MCP smoke to a manual check and rely on Task 9's **direct-socket** driver for CI (the MCP layer is a thin pass-through). Optionally: `ND_AUTOMATION_SOCKET=/nonexistent bun packages/mcp/src/index.ts` should exit with the "not set"/connect error — proves the entrypoint runs.

- [ ] **Step 5: Commit**
```bash
git add packages/mcp/package.json packages/mcp/tsconfig.json packages/mcp/src/socket.ts packages/mcp/src/index.ts bun.lock
git commit -m "feat: packages/mcp stdio mcp server wrapping the automation socket"
```

### Task 9: `scripts/headless-m4.sh` + `scripts/m4-drive.ts` + D11 SLO + CI + spec deviation note

**Files:**
- Create: `scripts/m4-drive.ts` (dependency-light direct-socket driver)
- Create: `scripts/headless-m4.sh`
- Modify: `.github/workflows/ci.yml` (append one step)
- Modify: `docs/superpowers/specs/2026-07-09-nativedesktop-design.md` (§8: absorb the transport deviation)

**Interfaces:**
- Consumes: the Task 1–6 host, the Task 7 tagged demo, weston (M1 flake).
- Produces: an end-to-end headless proof (drive counter via the socket) + the D11 SLO proof, run by CI.

- [ ] **Step 1: Write `scripts/m4-drive.ts` (no deps beyond Bun + the framing)**

Reuse `AutomationClient` from `packages/mcp/src/socket.ts`? Keep the driver dependency-light and standalone — inline a minimal framed client (or import the socket module by relative path; importing is fine and shares no npm dep). The driver:
```ts
// scripts/m4-drive.ts — connects ND_AUTOMATION_SOCKET, drives the counter, asserts, screenshots.
import { AutomationClient } from "../packages/mcp/src/socket.ts";

const outPng = process.env.M4_PNG ?? "/tmp/m4-shot.png";
const client = await AutomationClient.connect();

// find the increment button by testID
type Node = { ref: number; type: string; testID: string | null; text: string | null; children: Node[] };
function find(n: Node, testID: string): Node | null {
  if (n.testID === testID) return n;
  for (const c of n.children) { const f = find(c, testID); if (f) return f; }
  return null;
}
const tree = (await client.call("getTree")) as { root: Node; coordinateSpace: string };
if (tree.coordinateSpace !== "logical-window-topleft") throw new Error("bad coordinate space");
const btn = find(tree.root, "increment-button");
if (!btn) throw new Error("increment-button not found in tree");

for (let i = 0; i < 3; i++) await client.call("click", { ref: btn.ref });
await client.call("waitFor", { condition: { textContains: "Clicks: 3" }, timeoutMs: 3000 });

const shot = (await client.call("screenshot", { path: outPng })) as { path: string; width: number; height: number };
if (shot.width <= 0 || shot.height <= 0) throw new Error("screenshot has no dimensions");
console.log(`M4_DRIVE_OK clicks=3 png=${shot.path} ${shot.width}x${shot.height}`);
```
The `waitFor` success (no throw) proves clicks reached GTK and re-rendered the label.

- [ ] **Step 2: Write `scripts/headless-m4.sh` (unique weston socket; launch counter + drive + SLO)**

Mirror `headless-m3.sh`/`kill9-test.sh`. Launch the host with `NATIVE_AUTOMATION=1` in the background, wait for `ND_AUTOMATION_LISTENING` and `ND_HELLO_OK`, extract the automation socket path from the marker line, export `ND_AUTOMATION_SOCKET`, run the driver, then the SLO test.
```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
export WAYLAND_DISPLAY=nd-headless-m4
export GSK_RENDERER=cairo
export GDK_BACKEND=wayland
export ND_SCRIPT=examples/counter/main.tsx
export NATIVE_AUTOMATION=1

weston --backend=headless --socket="$WAYLAND_DISPLAY" --idle-time=0 &
WESTON_PID=$!
trap 'kill "$WESTON_PID" 2>/dev/null || true; kill "$HOST_PID" 2>/dev/null || true' EXIT
for _ in $(seq 1 50); do [ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ] && break; sleep 0.1; done

LOG=$(mktemp)
./zig-out/bin/nd-hello >"$LOG" 2>&1 &
HOST_PID=$!

# Wait for the automation listener + the react handshake + the first commit (so widgets exist).
for _ in $(seq 1 80); do grep -q "ND_AUTOMATION_LISTENING" "$LOG" && grep -q "ND_COMMIT_APPLIED" "$LOG" && break; sleep 0.1; done
grep -q "ND_AUTOMATION_LISTENING" "$LOG" || { echo "FAIL: no automation listener"; cat "$LOG"; exit 1; }
SOCK=$(grep -m1 "ND_AUTOMATION_LISTENING" "$LOG" | sed 's/.*path=//')
export ND_AUTOMATION_SOCKET="$SOCK"

# Drive the app via the socket directly.
M4_PNG="$XDG_RUNTIME_DIR/m4-shot.png" bun scripts/m4-drive.ts || { echo "FAIL: driver"; cat "$LOG"; exit 1; }
[ -s "$XDG_RUNTIME_DIR/m4-shot.png" ] || { echo "FAIL: empty png"; exit 1; }
file "$XDG_RUNTIME_DIR/m4-shot.png" | grep -q "PNG image" || { echo "FAIL: not a png"; exit 1; }

# ---- D11 SLO: stall the bun child, automation must still answer within 1s ----
BUN_PID=$(pgrep -P "$HOST_PID" -f "bun" | head -1)
[ -n "$BUN_PID" ] || { echo "FAIL: no bun child for SLO test"; exit 1; }
kill -STOP "$BUN_PID"
SLO_PNG="$XDG_RUNTIME_DIR/m4-slo.png"
timeout 1 bash -c "ND_AUTOMATION_SOCKET='$SOCK' M4_SLO_PNG='$SLO_PNG' bun scripts/m4-slo.ts" \
  || { echo "FAIL: automation did not answer within 1s while child stalled"; kill -CONT "$BUN_PID"; cat "$LOG"; exit 1; }
kill -CONT "$BUN_PID"

kill -TERM "$HOST_PID"; wait "$HOST_PID" 2>/dev/null || true
echo "headless m4: OK (drove counter, screenshot written, D11 SLO under stall)"
```
`scripts/m4-slo.ts` is a tiny script (create it in this step) that only does `getTree` + `screenshot` and prints `M4_SLO_OK` — proving both answer while the child is SIGSTOPped. (Split from the driver so the SLO run is minimal and time-bounded by `timeout 1`.)

- [ ] **Step 3: Write `scripts/m4-slo.ts`**
```ts
import { AutomationClient } from "../packages/mcp/src/socket.ts";
const client = await AutomationClient.connect();
const tree = await client.call("getTree");
if (!tree) throw new Error("no tree under stall");
await client.call("screenshot", { path: process.env.M4_SLO_PNG! });
console.log("M4_SLO_OK");
```

- [ ] **Step 4: Run the full M4 gate locally**

Run:
```bash
nix develop -c bash -c 'bun install && zig build && chmod +x scripts/headless-m4.sh && ./scripts/headless-m4.sh'
```
Expected: `M4_DRIVE_OK clicks=3 …`, `M4_SLO_OK`, then `headless m4: OK …`, exit 0. The SLO sub-run must complete inside `timeout 1` — if it times out, the marshal-and-block path is waiting on something the child holds (a bug: it must not). Debug per the systematic-debugging skill; one hypothesis at a time. Fix the first failure.

- [ ] **Step 5: Extend CI**

Append after the `headless m3` step in `.github/workflows/ci.yml`:
```yaml
      - name: headless m4
        run: nix develop -c ./scripts/headless-m4.sh
```
(`bun install --frozen-lockfile` already runs before `headless m3`; the M4 script reuses that install. If `packages/mcp` or the driver needs deps not yet installed by the frozen step, the frozen install already covers them since `bun.lock` was updated in Task 8.)

- [ ] **Step 6: Absorb the transport deviation into the spec**

In `docs/superpowers/specs/2026-07-09-nativedesktop-design.md` §8, append a short paragraph after the "Surface: JSON-RPC over a local WebSocket + stdio…" sentence recording that **M4 v1 ships the JSON-RPC surface over a second framed unix socket (NDP framing) rather than WebSocket; WebSocket is deferred; the first-party MCP wrapper (`packages/mcp`) provides the stdio+MCP half of the surface and bridges to the socket.** One paragraph, no restructuring. This is the only spec edit; do not touch other sections.

- [ ] **Step 7: Validate the full CI sequence locally, then commit**

Run:
```bash
nix develop -c bash -c 'zig build test && zig build && ./scripts/headless-smoke.sh && ./scripts/headless-m2.sh && ./scripts/kill9-test.sh && bun install --frozen-lockfile && ./scripts/headless-m3.sh && ./scripts/headless-m4.sh'
```
Expected: every stage green — the exact CI sequence plus M4. Then:
```bash
git add scripts/m4-drive.ts scripts/m4-slo.ts scripts/headless-m4.sh .github/workflows/ci.yml docs/superpowers/specs/2026-07-09-nativedesktop-design.md
git commit -m "ci: headless m4 drives the counter via automation socket with d11 slo test"
```

---

## Self-review notes

**M4 scope coverage (spec §14 M4 + owner contract, not expanded):**
- JSON-RPC automation server in the Zig host behind `NATIVE_AUTOMATION=1` on a second framed unix socket — Task 2 (listener thread, accept-one-client, `encodeFrame`/`readFrame` reuse, `ND_AUTOMATION_*` markers), Task 2 Step 3 (env-gated start). Deliberate WebSocket→unix-socket deviation recorded in the Architecture block and written into the spec in Task 9 Step 6.
- `getTree` snapshot with stable refs, widget type, `testID`, text, visibility, logical geometry + stated coordinate-space contract, served on the UI thread even under a stalled child — Task 3 (traversal + `computeBounds` to the window root, `coordinateSpace:"logical-window-topleft"`), D11 mechanism in Task 2 Step 2.
- `screenshot` in-process GTK→GSK render to a caller path returning `{path,width,height}` — Task 4, using the verified `getNative`→`getRenderer`→`Snapshot`/`WidgetPaintable`→`renderTexture`→`saveToPng` chain (fallback cairo renderer + fallback snapshot route both specified with verify steps).
- `click` semantic dispatch with a mandatory actionability hit-test (exists∧visible∧mapped∧non-degenerate bounds; `-32001` on failure), emit `clicked` via `activate`/`signalEmitByName` — Task 5 (encodes the research gotcha "never click what a user couldn't").
- `setValue`/`type`/`scroll` signatures defined, returning `-32601` "not implemented until M5 widgets" — Task 6.
- `waitFor {condition:{textContains|refVisible}, timeoutMs}` polling the tree on the UI thread at ~50 ms — Task 6.
- `testID` string prop on every widget, plumbed JS-side, stored host-side, echoed in `getTree`, never applied to GTK — Task 1 (host `NodeMeta.test_id`), Task 7 (intrinsic prop + demo tags), with the explicit note that `emitCreateIfNew` already forwards it (no host-config code change).
- First-party MCP wrapper (`packages/mcp`, Bun, stdio MCP) exposing `nd_get_tree`/`nd_screenshot`/`nd_click`/`nd_wait_for`, connecting via `ND_AUTOMATION_SOCKET` — Task 8, using `@modelcontextprotocol/sdk` (decision justified: spec-correct stdio handshake + schema for free vs. a hand-roll that only saves one dep on an agent-facing surface).
- Demo/CI: `scripts/headless-m4.sh` + dependency-light `scripts/m4-drive.ts` (getTree→find-by-testID→click×3→waitFor "Clicks: 3"→screenshot→assert non-empty PNG) + the D11 SLO (SIGSTOP the child, `getTree`+`screenshot` answer within 1s via `m4-slo.ts` under `timeout 1`, SIGCONT, clean exit); CI step appended — Task 9.

**Architecture constraints honored (owner threading decisions):** the automation server is its **own listener thread** accepting **one client at a time** (Task 2 Step 1); every widget-touching request marshals to the GTK main thread via the **same `glib.MainContext.default().invokeFull` pattern `runtime.zig` already uses** and blocks the automation thread on a `std.Io.Condition`+`std.Io.Mutex` handoff (Task 2 Step 2 — the exact `std.Io` primitives already in `runtime.zig`); **tree access is UI-thread-only with no new tree mutex** (the mutex/condition guard only the request/response struct, not the tree); the automation thread never calls GTK directly. The D11 SLO holds because the marshaled UI callback runs on the GLib loop, which ticks independent of the Bun child — reads of `tree.meta` + GTK geometry need no child cooperation.

**Landed-code fidelity:** every host reference matches the landed files — `Tree.nodes` is `AutoHashMapUnmanaged(u32,*gtk.Widget)` with **no** type/testID/text today (Task 1 adds `meta`); the `create`/`append`/`setText`/`update`/`remove` arms and their exact prop access (`propStr`, `op.props: ?std.json.Value`) are cited by file:line; the socket/`invokeFull`/`std.Io.Mutex` patterns are reused verbatim from `runtime.zig`; `the_window` is the screenshot/geometry root via a new `getWindow` getter; `testID` flows through the existing `emitCreateIfNew` prop copy with no host-config change; the JSX intrinsics live in `jsx-runtime.ts` (not a `jsx.d.ts`, per the M3 landed reality). New markers `ND_AUTOMATION_LISTENING/CONNECTED/DISCONNECTED` and `ND_RPC` are emitted where the headless-m4 script greps for them.

**Every new GTK/GSK/GDK/std symbol is verified** in the verified-symbol table with the exact `rg` command and this-session line number — `gtk_native_get_renderer`, `gtk_snapshot_new`/`free_to_node`, `gtk_widget_snapshot_child`, `gtk_widget_paintable_new`, `gdk_paintable_snapshot`, `gsk_renderer_render_texture`, `gsk_cairo_renderer_new`/`gsk_renderer_realize`, `gdk_texture_save_to_png`, `gsk_render_node_unref`, `gtk_widget_compute_bounds`, `gtk_widget_get_root`/`_width`/`_height`/`_visible`/`_mapped`/`_native`/`_first_child`/`_next_sibling`, `gtk_widget_activate`, `g_signal_emit_by_name`, `graphene.Rect{f_origin,f_size}`, `g_main_context_invoke_full`, `std.Io.Condition`/`std.Io.Mutex`/`std.Io.net.Server.accept`. Where headless behavior is uncertain (which renderer `getRenderer` returns under cairo; whether `activate` vs `signalEmitByName` drives the button; which snapshot route yields a non-empty PNG), the plan embeds a verify-at-implement-time step and records the working choice rather than guessing.

**Parallelization:** the header note marks the Zig automation-server track (Tasks 1–6) and the JS+MCP track (Tasks 7–8) as file-disjoint and concurrently runnable; Task 9 integrates and must run last. Within the Zig track, Task 2 depends on Task 1, and Tasks 3–6 (the four handlers) depend on Task 2 and are mutually parallel.

**Judgment calls made within the constraints:**
1. **Transport = second framed unix socket, not WebSocket** (owner-approved deviation) — reuses the proven NDP framing/accept stack for zero new deps; the spec is amended in Task 9 Step 6. WebSocket deferred.
2. **MCP wrapper uses `@modelcontextprotocol/sdk`** (verified 1.29.0) rather than hand-rolled stdio JSON-RPC — a spec-correct handshake + tool schemas for free on an agent-facing deliverable outweigh saving one dependency; the only bespoke code is the ~40-line framed socket client.
3. **`getTree` child ordering via the live GTK child-walk** (`getFirstChild`/`getNextSibling` + a `widget→id` reverse map) preferred over `meta.parent` insertion-order grouping — gives true visual order and survives reorders; `meta.parent` is the fallback.
4. **`testID` stored in a parallel `meta` map, not applied to GTK** — matches the owner constraint (stored + echoed, not `gtk_widget_set_name`); a11y-tree mirroring is an M5+ concern. `emitCreateIfNew` already forwards `testID` to the wire, so the JS side is a type-only change plus two demo tags.
5. **Actionability = existence+visible+mapped+non-degenerate-bounds** for M4's 4-widget set; full z-order/overlap coverage-testing is deferred (documented in Task 5 Step 1) — the achievable subset that still refuses to click invisible/unmapped/zero-size widgets, honoring the research gotcha.
6. **D11 SLO split into `m4-slo.ts` under `timeout 1`** (getTree+screenshot only, child SIGSTOPped) — the minimal, time-bounded proof that the automation path never blocks on the child; the full driver stays a separate, un-stalled run.
7. **`click` emit via `gtk.Widget.activate` primary, `signalEmitByName` fallback** — both verified-present; the reliable-under-weston choice is confirmed by asserting `Clicks: 3` in Task 9 and recorded.
8. **Screenshot via `WidgetPaintable` primary, `snapshotChild` fallback** — the paintable route takes the widget directly (clean full-window capture); the child-snapshot route is the fallback if the `gtk.Snapshot`→`gdk.Snapshot` cast doesn't hold. Renderer availability under cairo is verified at implement-time with the fallback cairo renderer specified.
