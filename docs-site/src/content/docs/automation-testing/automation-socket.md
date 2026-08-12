---
title: Automation Socket
description: The full JSON-RPC automation surface, covering transport, methods, error codes, and known gaps.
---

`schema/rpc.json` is the ground truth for this page. `tools/codegen.ts` generates both
`src/generated/rpc.zig` (consumed by `src/automation.zig`) and
`packages/react/src/generated/rpc.ts` from it, and this page is a readable mirror. If the two
disagree, the schema and its generated output win. Changing a method, param, or result in the schema
is a compile error on both the Zig and the TypeScript side until every caller is updated, never a
silent runtime break.

## Transport

Framed JSON-RPC 2.0: a `u32` little-endian length prefix followed by the UTF-8 JSON payload, one
frame per message, over a unix domain socket at `$XDG_RUNTIME_DIR/nd-automation-<host-pid>.sock`.
The host prints the socket path as `ND_AUTOMATION_LISTENING path=<path>` on stderr once it's ready.
The server is gated on `NATIVE_AUTOMATION=1` in the host's environment; if it's unset, the socket
never opens. `packages/mcp` is a stdio MCP server that bridges this socket to MCP tool calls; see
[MCP Tools](/automation-testing/mcp-tools/). For writing a Bun/TypeScript test or drive script
against this socket directly — launching the host, connecting, and querying/acting/waiting on the
tree — `@nativedesktop/test` wraps the raw RPC calls documented on this page into a
`launchApp`/`AppHandle` API; see [Test Harness](/automation-testing/test-harness/).

## Methods

| Method | Params | Result | Notes |
|---|---|---|---|
| `getTree` | `{window?}` | `{coordinateSpace, root: JsonNode}` | accessibility-tree snapshot; `window` (a Window node ref) scopes it to that window's subtree |
| `screenshot` | `{path, window?}` | `{path, width, height}` | in-process render → PNG; `window` picks any open window's Window node ref |
| `click` | `{ref?, testId?, window?, action?}` | `{ref, dispatched: true}` | actionability-checked, emits `clicked` semantically; on `CommandPalette` activates the currently-highlighted row; `action` invokes a SourceTree row action (see below) |
| `waitFor` | `{condition: WaitCondition, timeoutMs?, window?}` | `{matched, ref, count}` | polls at ~50ms; default `timeoutMs` 2000; see [waitFor conditions](#waitfor-conditions) |
| `setValue` | `{ref?, testId?, window?, value}` | `{ref, applied: true}` | kind-dispatched: `TextInput`/`TextArea` need a string, `Checkbox`/`Radio` a bool, `Slider` a number, `Select` an integer index |
| `type` | `{ref?, testId?, window?, text}` | `{ref, text: <full text after insert>}` | `TextInput` only; semantic append via `GtkEditable.insertText`, never synthetic keysyms |
| `scroll` | `{ref?, testId?, window?, dx?, dy?}` | `{ref, x, y}` (resulting adjustment values) | `ScrollView` only |
| `doubleClick` | `{ref?, testId?, window?}` | `{ref, dispatched: true}` | real double-click at the widget's center (activates table/list rows); macOS only |
| `rightClick` | `{ref?, testId?, window?}` | `{ref, dispatched: true}` | real right-click at the widget's center; an opened context menu is auto-dismissed with an escape (see below); macOS only |
| `hover` | `{ref?, testId?, window?}` | `{ref, dispatched: true}` | best-effort `mouseMoved` at the widget's center; macOS only |
| `resolve` | `{testId, window?, actionable?}` (default `actionable: true`) | `{ref, refs, count}` | resolves a testID to the single ACTIONABLE instance, ranked actionable-first then key/front-window-first; `refs` is every match in tree order, `ref` the winner (`null` when none is actionable) |
| `windows` | `{}` | `{windows: WindowInfo[]}` | every open Window node's live `{ref, title, key, main, visible, tabGroup}`, in tree order |
| `pointer` | `{phase: "down"\|"move"\|"up", x, y, button?, clickCount?, window?}` | `{dispatched: true}` | low-level single pointer phase at window-topleft coordinates; `clickCount: 2` on a down/up pair makes a double-click; macOS only |
| `drag` | `{fromRef?\|fromX?,fromY?, toRef?\|toX?,toY?, steps?, durationMs?, button?, window?}` | `{dispatched, fromX, fromY, toX, toY, steps}` | press-move-release; ref endpoints resolve to widget centers and must share a window; macOS only |
| `keys` | `{keys, window?}` | `{dispatched: true}` | `"cmd+shift+n"` presses one chord (drives menu key equivalents); `"escape"`/`"tab"` a named key; any other string types its characters into the focused widget; macOS only |

`click`, `setValue`, `type`, `scroll`, `doubleClick`, `rightClick`, and `hover` all target by **exactly
one of `ref` or `testId`** (`invalidParams` otherwise); `window` optionally scopes `testId`
resolution to one window, using the same actionable-first ranking as `resolve`. Targeting by
`testId` is one round trip with host-side resolution — no `getTree` walk needed first.

### SourceTree row actions

`click` with `action` invokes a `<sourcetree>` row's trailing action semantically, dispatching
`actionClicked {nodeId, actionId}` exactly like a real click on the row's button. This is the path
to use when a flow's affordance is a row action (hover-only buttons included), and the only one on
GTK (no input synthesis). `testId` may be a **row's** per-node testID (SourceTree rows are meta
data, not tree nodes, so the host resolves a row testID to its owning widget and the backend picks
the row) or the widget's own `testId`/`ref`, in which case the currently-selected row is the target.
The row must be realized under the current expansion (a collapsed ancestor makes it unreachable,
like for a user) and must declare the action in its `actionIds`; otherwise the call answers
`invalidParams` (-32602). `@nativedesktop/test`: `app.click({ testId: "row-testid", action:
"action-id" })`.

`JsonNode` (from `getTree`, nested under `root`/`children`):
`{ref, type, testID, text, visible, geometry: {x,y,w,h} | null, children, itemCount, rows, role,
enabled, focused, value}`. `itemCount` is non-null only for data-driven widgets (currently
`ListView`): it's the row count, never a walk of recycled row widgets. `rows` is non-null only for
row-driven widgets (currently `SourceList`) and carries each row's
`{title, badge: string | null, iconName: string | null}`.

The last four fields are the accessibility-tree state: `role` is the widget's schema-declared
automation role from `schema/widgets.json` (`"button"`, `"slider"`, `"window"`, …; null when the
type declares none); `enabled`/`focused`/`value` come from a live per-node backend probe on every
snapshot. `value` is kind-shaped exactly like `setValue`'s input: string for `TextInput`/`TextArea`,
boolean for `Checkbox`/`Radio`/`Switch`, number for `Slider`, selected index for `Select` and
row-selection widgets (`SourceList`/`Table`/`TreeView`), null for widgets without a value. Backends
without the probe degrade to the defaults (`enabled: true`, `focused: false`, `value: null`) rather
than failing the snapshot.

## waitFor conditions

`WaitCondition` is a flat struct with exactly one selector — `textContains`, `refVisible`, or
`testId` — evaluated host-side against the retained tree plus the live a11y probe, once per ~50ms
tick. `waitFor` never does a `getTree` round trip internally; it's the same poll loop the tree
itself is built from.

```ts
interface WaitCondition {
  textContains?: string;
  refVisible?: number;
  testId?: string;
  state?: "present" | "gone" | "visible" | "enabled" | "disabled" | "focused"; // default "present"
  countAtLeast?: number;
  valueEquals?: string;
  valueContains?: string;
}
```

With `testId`, `state` picks the predicate (default `"present"`: the node exists in the tree at
all):

- `"gone"` — no node with this testID exists. A testID that never existed satisfies `"gone"`
  immediately.
- `"visible"` / `"enabled"` / `"focused"` — the node exists and its `visible`/`enabled`/`focused`
  a11y field is `true`.
- `"disabled"` — the node exists and `enabled` is `false`.

`countAtLeast` and `valueEquals`/`valueContains` refine the match:

- `countAtLeast` counts tree nodes whose testID equals `testId` — for `state` other than
  `"present"`, only actionable ones count (the same rule `resolve` uses). Most testIDs are unique
  per node, so this is usually a 0-or-1 check; it matters when an app deliberately reuses one
  testID across N rendered items.
- `valueEquals`/`valueContains` compare against the node's a11y `value`, rendered as a **string**
  (numbers stringified, booleans `"true"`/`"false"`) — so one predicate works for a `TextInput` and
  a `Slider` alike. For `TextInput`/`TextArea`, `value` mirrors `text`, so `valueContains` is a
  testID-scoped alternative to a global `textContains` search.

`WaitForResult` is `{matched, ref, count}`: `ref` is the winning match (ranked like `resolve` —
actionable first, key window first, then tree order), so a caller needs no follow-up `getTree`;
`count` is how many nodes satisfied the predicate. A timeout is a JSON-RPC error (`-32002`), not a
`matched: false` result — see [Error codes](#error-codes).

## Input synthesis by platform

`pointer`, `drag`, `keys`, `doubleClick`, `rightClick`, and `hover` post real native events on
macOS: constructed `NSEvent`s pushed through the app's own event queue with `NSApp.postEvent`.
Slider thumbs, split-view dividers, table row activation, text selection, and menu key equivalents
all run genuine AppKit machinery, in-process and without any TCC permission. Multi-event gestures
like `drag` and double-clicks are posted as one batch, because AppKit controls run nested
mouse-tracking loops inside `mouseDown` dispatch that consume the rest of the gesture from the queue
while blocking the main thread.

On GTK these methods answer `-32003`, `input synthesis unsupported on this backend`. GTK4 removed
app-constructible `GdkEvent`s, so in-process synthesis is impossible. Use the semantic methods
`click`, `setValue`, `type`, and `scroll` on Linux instead. The accessibility fields work on both
backends.

Three behaviors are deliberate:

- A lone `pointer` `down` on a tracking control (slider, button) enters that control's
  mouse-tracking loop until an `up` event arrives; prefer `drag` for press-move-release sequences.
- `rightClick` auto-appends an escape key press: a context menu's tracking mode does not service
  the main dispatch queue, so a menu left open would wedge every later automation call. The menu
  still opens and closes for real (its open/close hooks run); menu contents are not yet in the
  tree.
- Coordinate clicks land wherever a real click would: a leading-aligned control whose frame is
  stretched by its container (a checkbox in a full-width column, say) only reacts over its visible
  glyph/label region on macOS. Aim at the leading edge of `geometry` rather than the center, or use
  the semantic `click`/`setValue`, which don't depend on coordinates at all.

## Error codes

| Code | Meaning | `data` shape |
|---|---|---|
| `-32001` | not actionable | `{ref, reason: "unknown" \| "invisible" \| "unmapped" \| "offscreen"}` |
| `-32002` | `waitFor` timed out | `{timeoutMs}` |
| `-32003` | input synthesis unsupported on this backend | `{ref}`, raised by pointer/drag/keys/doubleClick/rightClick/hover on GTK |
| `-32601` | method not found | none |
| `-32602` | invalid params | `{ref}` where applicable |
| `-32603` | internal error | none, or a message describing the failure |
| `-32700` | parse error | none |

Actionability (`-32001`) is checked before every action-dispatch method (`click`, `setValue`,
`type`, `scroll`): the ref must exist, be visible, be mapped, and have non-degenerate on-screen
bounds relative to the window. The checks mirror what a real user could reach.

## Coordinate space

`coordinateSpace` is always `"logical-window-topleft"`: every `geometry` field in `getTree` is in
logical units (not device pixels), relative to the window's top-left corner.

## Known gaps

- `scroll` only targets `ScrollView`-typed nodes. A `ListView` node can't be scrolled directly;
  scroll its wrapping `ScrollView` if one exists.
- No `TabView` page-switch RPC. A widget on a non-active tab page is `visible: false` (GTK/AppKit
  both unmap/hide inactive pages), so it fails the actionability check for `click`/`setValue`/etc.
  regardless of whether it's targeted by `ref` or `testId` — testID targeting resolves the node,
  it doesn't make a hidden one actionable. Put automation-driven controls on the default-active
  (first) tab, or outside the `TabView` entirely, until this lands.
- No `ListView` row-activate/select action from the RPC side (the widget emits `onRowActivated`
  upward to React, but there's no automation method to trigger it).
- A post-scroll `screenshot` can occasionally race frame invalidation and return a stale texture.
  Retry (poll every ~150ms, up to ~3s) rather than treating one blank shot as final.
- An empty `TextArea` collapses to 0 logical height, so it fails the actionability check until it
  has content or explicit sizing.
- Prefer `setValue({ref, value: boolean})` over `click` for `Checkbox`/`Radio`: `click` toggles the
  current state (relative), while `setValue` sets an exact, deterministic state.
- **GTK cannot rasterize live WebKit content into a screenshot.** Full WebKit rasterization is out
  of scope: the render-to-texture path fails whenever WebKit hands the compositor a texture the
  snapshot renderer can't download (headless cairo fails outright; GL only works for a webview
  sitting directly in the window; DMABUF on a real GPU fails for both). When the pixel-true pass
  works (WebKit rendering in software/SHM mode, common under headless weston once a page has
  settled), the screenshot carries real web content. When it fails and webviews are in the window,
  the snapshot **degrades instead of erroring**: each `<webview>` region is painted as a flat gray
  plate with a centered "WebView" label, the rest of the window renders normally, and the host
  prints `ND_SNAPSHOT_DEGRADED webviews=N` on stderr (the frozen ABI's `snapshot` op returns only
  a bool, so the marker is the machine-readable signal). Windows containing webviews (browser,
  multiwindow) are therefore always capturable; treat a degraded capture as layout-true but not
  pixel-true for the web content itself.

## Crash/overlay contract

After a runtime crash or disconnect, the host paints an in-window overlay and registers its chrome
widgets in the tree under a reserved generation (`0xFF`), so `getTree` keeps answering through the
crash. Because a JS crash is one Bun process dying, the overlay is painted on every open window.
The registered testIDs are `nd-overlay-panel`, `nd-overlay-title`, `nd-overlay-error`, and
`nd-overlay-restart` (the Restart button); agents can `waitFor` them and drive recovery. Both
backends implement it (`src/gtk/overlay.zig`, `swift/Sources/NDShell/Overlay.swift`).

## Scripted native dialogs

Native file pickers, save panels, and alerts (`NSOpenPanel`/`GtkFileDialog`, `NSAlert`/
`AdwAlertDialog`) are real modal OS UI — an automated run can't click through them. Setting
`ND_AUTOMATION_DIALOG_SCRIPT` (honored only when `NATIVE_AUTOMATION=1`) answers them from a
per-method FIFO instead of ever opening the real dialog:

```json
{
  "dialog.openFile": [["/tmp/a.txt"], []],
  "dialog.saveFile": ["/tmp/out.txt", null],
  "dialog.showMessage": [0],
  "window.showAlert": [{ "buttonId": "delete" }],
  "window.openFile": [{ "canceled": false, "paths": ["/tmp/a.txt"] }],
  "window.saveFile": [{ "canceled": true, "path": null }]
}
```

The value is inline JSON or `@/path/to.json`. Two interception points, matching the two ways an app
can show a dialog:

- **App-level** (`dialog.openFile`/`dialog.saveFile`/`dialog.showMessage` — `system.ts`'s `dialog`
  object, a `systemRequest`): each FIFO entry is the **raw** value the JS call resolves to —
  `dialog.openFile` entries are `string[]` (chosen paths, `[]` for canceled), `dialog.saveFile`
  entries are `string | null`, `dialog.showMessage` entries are the clicked button's index.
- **Window-scoped** (`window.showAlert`/`window.openFile`/`window.saveFile` — `dialogs.ts`'s
  `showAlert`/`openFile`/`saveFile`, a `widgetCommand` on a `<window>` node): each entry is the
  real result shape the promise resolves to — `AlertResult {buttonId}`,
  `OpenFileResult {canceled, paths}`, `SaveFileResult {canceled, path}`.

FIFO per method: each call shifts the head. An **exhausted queue never falls through to the real
dialog** — it fails loudly instead. App-level exhaustion rejects the pending `systemRequest`
(`ok: false`, `"dialog script exhausted: <method>"`); window-scoped exhaustion has no promise to
reject (there's no `*Result` event to synthesize), so it prints `ND_DIALOG_SCRIPT_EXHAUSTED
method=<name>` on stderr instead — poll for that marker (`AppHandle.waitForMarker` in
[Test Harness](/automation-testing/test-harness/)) to assert the drain.

## Screenshots on macOS (ndshot)

The `screenshot` RPC renders offscreen inside the host process, and on macOS 26 that path draws
blank editable fields for `TextInput`/`TextArea` (`_NSCoreHostingView` only paints via CoreAnimation
when actually composited on screen). `tools/ndshot/` works around this: a small, dependency-free
Swift package with its own stable binary identity that preflights/requests Screen Recording once,
then captures the live composited window via ScreenCaptureKit. Capture works even when the window
is occluded.

```bash
cd tools/ndshot && ./build.sh
```

Three subcommands under `.build/release/ndshot`: `doctor` (reports Screen Recording permission
state; exit 0 granted, 2 not), `list` (enumerates capturable windows as JSON lines), and
`capture --out <path.png> [--pid <pid>] [--title <substring>] [--window-id <id>]`. The first
invocation of `list`/`capture` triggers the system's one-time, headful Screen Recording permission
prompt. Grant it once via System Settings → Privacy & Security → Screen Recording; the grant then
sticks to this binary's path and ad hoc signature across future runs and rebuilds. A rebuild that
changes the binary's bytes counts as a new identity and needs re-granting.

Freshness contract: ScreenCaptureKit samples the **window server's composite**, not the live view
tree, so a commit AppKit has applied but never displayed would capture stale (measured on the
panes example: a status label reached the composite while a freshly inserted `NSSplitView` subtree
stayed blank until the RPC ladder's own `displayIfNeeded` healed it). `NATIVE_AUTOMATION=1` hosts
therefore run a ~100ms main-runloop tick that forces `layoutSubtreeIfNeeded`/`displayIfNeeded` on
every visible window plus a `CATransaction.flush`, and the in-process SCK rung
(`ND_AUTOMATION_CAPTURE=screencapturekit`) flushes the same way immediately before capturing. An
external `ndshot` capture of an automation host is current as of the last tick (well inside its
own ~250ms frame-stability resample); capturing a NON-automation app has no such guarantee.
