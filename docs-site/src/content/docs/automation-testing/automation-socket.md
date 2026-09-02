---
title: Automation Socket
description: The full JSON-RPC automation surface, covering transport, methods, error codes, and known gaps.
---

`schema/rpc.json` is the ground truth for this page. `tools/codegen.ts` generates both
`src/generated/rpc.zig` (consumed by `src/automation.zig`) and
`packages/react/src/generated/rpc.ts` from it, and this page mirrors them. If the two disagree, the
schema and its generated output win. Changing a method, param, or result in the schema is a compile
error on both the Zig and the TypeScript side until every caller is updated, never a silent runtime
break.

## Transport

Framed JSON-RPC 2.0: a `u32` little-endian length prefix followed by the UTF-8 JSON payload, one
frame per message, over a unix domain socket at `$XDG_RUNTIME_DIR/nd-automation-<host-pid>.sock`.
The host prints the socket path as `ND_AUTOMATION_LISTENING path=<path>` on stderr once it is ready.
The server is gated on `NATIVE_AUTOMATION=1` in the host's environment; unset, the socket never
opens.

Two clients wrap this socket. `packages/mcp` is a stdio MCP server that bridges it to MCP tool
calls; see [MCP Tools](/automation-testing/mcp-tools/). `@nativedesktop/test` wraps the raw RPC
calls on this page into a `launchApp`/`AppHandle` API for Bun tests and drive scripts; see
[Test Harness](/automation-testing/test-harness/).

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
| `windows` | `{}` | `{windows: WindowInfo[]}` | every open Window node's live `{ref, title, key, main, visible, tabGroup, geometry}`, in tree order; `geometry` is the window frame in the same logical top-left units node geometry uses (`w`/`h` are width/height), and is `null` on a backend whose probe omits it |
| `pointer` | `{phase: "down"\|"move"\|"up", x, y, button?, clickCount?, window?}` | `{dispatched: true}` | low-level single pointer phase at window-topleft coordinates; `clickCount: 2` on a down/up pair makes a double-click; macOS only |
| `drag` | `{fromRef?\|fromX?,fromY?, toRef?\|toX?,toY?, steps?, durationMs?, button?, window?}` | `{dispatched, fromX, fromY, toX, toY, steps}` | press-move-release; ref endpoints resolve to widget centers and must share a window; macOS only |
| `keys` | `{keys, window?}` | `{dispatched: true}` | `"cmd+shift+n"` presses one chord (drives menu key equivalents); `"escape"`/`"tab"` a named key; any other string types its characters into the focused widget; macOS only |
| `webviewInfo` | `{ref?, testId?, window?}` | `{ref, url, title, loading, canGoBack, canGoForward}` | live page state read off the engine; the target must be a `WebView` (`invalidParams` otherwise) |
| `webviewEval` | `{ref?, testId?, window?, code, world?, timeoutMs?}` | `{ref, ok, value, error}` | evaluates `code` in the page, optionally in a named isolated world; default `timeoutMs` 5000 |
| `focus` | `{ref?, testId?, window?}` | `{ok: true}` | moves keyboard focus to the target, the same path the widget-level `focus` command takes; afterwards `waitFor {testId, state: "focused"}` holds |
| `scrollIntoView` | `{ref?, testId?, window?}` | `{ok: true, scrolled}` | scrolls the target's nearest scrollable ancestor until the target is inside the viewport; `scrolled: false` means there was no scroll ancestor, which is a success |
| `snapshotNode` | `{ref?, testId?, window?, path?}` | `{path, width, height}` | renders ONE node to a PNG; `width`/`height` are the node's logical size, so they match its `geometry` (a window `screenshot` is at the display's backing scale instead). Without `path` the host writes beside the automation socket and answers where |
| `setWindowFrame` | `{window?, x?, y?, width?, height?}` | `WindowInfo` | moves and/or resizes a window, omitted components unchanged; answers that window's `WindowInfo` re-probed after the move |

`click`, `setValue`, `type`, `scroll`, `doubleClick`, `rightClick`, `hover`, `focus`,
`scrollIntoView`, `snapshotNode`, `webviewInfo`, and
`webviewEval` all target by exactly one of `ref` or `testId` (`invalidParams` otherwise). `window` optionally scopes `testId` resolution
to one window, using the same actionable-first ranking as `resolve`. Targeting by `testId` is one
round trip with host-side resolution, so no `getTree` walk is needed first.

`webviewInfo`, `webviewEval`, `scrollIntoView` and `snapshotNode` are the exceptions to the
actionability check. They resolve a node the user could not reach: only visibility and bounds are
waived, and a node that does not exist still answers `-32001`. The two webview methods ask a page a
question rather than act on a widget, which is what lets a drive inspect an extension's background
page or a non-active tab, both of which live in hidden `Activity` subtrees. `scrollIntoView` and
`snapshotNode` waive it for the opposite reason: a node scrolled out of its viewport reports
invisible, and refusing there would refuse exactly the node `scrollIntoView` exists to bring back.
Every other action still refuses a node it cannot see.

### SourceTree row actions

`click` with `action` invokes a `<sourcetree>` row's trailing action semantically, dispatching
`actionClicked {nodeId, actionId}` exactly like a real click on the row's button. Use it whenever a
flow's affordance is a row action, hover-only buttons included. It is the only path on GTK, which
has no input synthesis.

`testId` may be a row's per-node testID (SourceTree rows are meta data rather than tree nodes, so
the host resolves a row testID to its owning widget and the backend picks the row), or the widget's
own `testId`/`ref`, in which case the currently selected row is the target. The row must be realized
under the current expansion, since a collapsed ancestor makes it unreachable for a user too, and
must declare the action in its `actionIds`. Otherwise the call answers `invalidParams` (-32602).

In `@nativedesktop/test`: `app.click({ testId: "row-testid", action: "action-id" })`.

`JsonNode` (from `getTree`, nested under `root`/`children`):
`{ref, type, testID, text, visible, geometry: {x,y,w,h} | null, children, itemCount, rows, role,
enabled, focused, value, checked, selected, expanded, placeholder, label, options}`.
`itemCount` is non-null only for data-driven widgets (currently
`ListView`): it's the row count, never a walk of recycled row widgets. `rows` is non-null only for
row-driven widgets (`SourceList` `items`, `SourceTree` `nodes`, `CommandPalette` `items`) and
carries each row's `{title, badge, iconName, testID, id, subtitle}` (every field but `title`
nullable). `id` and `subtitle` are the row's own identity and secondary line where the widget's item
type has them — that is what lets a drive name a palette row instead of counting to it.

The last four fields are the accessibility-tree state: `role` is the widget's schema-declared
automation role from `schema/widgets.json` (`"button"`, `"slider"`, `"window"`, …; null when the
type declares none); `enabled`/`focused`/`value` come from a live per-node backend probe on every
snapshot. `focused` means "this is its window's focus widget", not "this window is frontmost" —
so it still reads true under a headless compositor that never activates a window. Menu nodes report
their declared `enabled`, which is how a drive tells a greyed-out menu item from a live one. `value` is kind-shaped exactly like `setValue`'s input: string for `TextInput`/`TextArea`,
boolean for `Checkbox`/`Radio`/`Switch`, number for `Slider`, selected index for `Select` and
row-selection widgets (`SourceList`/`Table`/`TreeView`), null for widgets without a value. Backends
without the probe degrade to the defaults (`enabled: true`, `focused: false`, `value: null`) rather
than failing the snapshot.

The last six fields come from the same probe and are `null` on every node the field does not apply
to, so a locator can ask `isChecked`/`isSelected`/`isExpanded` of any node and still tell "false"
apart from "not that kind of thing":

| Field | Non-null on | Meaning |
|---|---|---|
| `checked` | `Checkbox`, `Radio`, `Switch`, `SwitchRow` | the on/off state, the same boolean `value` carries |
| `selected` | a node drawn as a row of a list, table or outline | whether that row is selected (the state lives on the row, not on the app's own widget) |
| `expanded` | `Expander` | whether the disclosure is open |
| `placeholder` | `TextInput`, `SearchInput` | the empty-field prompt |
| `label` | icon-only controls, images, boxed-list rows | the spoken label where it says something `text` does not: a tooltip, an accessibility label, or the row title |
| `options` | `Select`, `ComboBox`, `SegmentedControl` | the choices in index order, so `setValue`'s integer index can be aimed by name |

A widget whose content is drawn by a SwiftUI body publishes nothing AppKit can read back, so on
macOS `Row`/`SwitchRow` titles and `SegmentedControl` option titles are recorded on the hosted leaf
as the props are applied (`ndA11yLabel` / `ndA11yOptions`, NDShell/SwiftUILeaves.swift) rather than
read out of the view. GTK answers both from the widgets themselves.

`visible` intersects the node's frame with every clip between it and the window: each enclosing
scroll viewport, then the window itself. A row scrolled out of its list is `visible: false` (and so
fails actionability) even though it is still laid out; `scrollIntoView` is what makes it actionable
again. `geometry` stays the untransformed frame either way, so a caller can still see where the
node would be.

A `HeaderBar` measures the run its items occupy in the window toolbar, and a `ToolbarView` measures
the content it installed. Both tracked handles are holders that never enter the view hierarchy on
macOS, so measuring them directly reported `0x0` and invisible for chrome that was plainly on
screen. When AppKit has not attached a custom item's view yet, the header falls back to the whole
title-bar-plus-toolbar band, which is where those items are drawn.

## waitFor conditions

`WaitCondition` is a flat struct with exactly one selector (`textContains`, `refVisible`, or
`testId`), evaluated host-side against the retained tree plus the live a11y probe once per ~50ms
tick. `waitFor` never does a `getTree` round trip internally. It runs on the same poll loop the tree
is built from.

```ts
interface WaitCondition {
  textContains?: string;
  refVisible?: number;
  testId?: string;
  state?: "present" | "gone" | "visible" | "enabled" | "disabled" | "focused"; // default "present"
  countAtLeast?: number;
  valueEquals?: string;
  valueContains?: string;
  urlContains?: string;        // WebView only
  pageTitleContains?: string;  // WebView only
  pageTextContains?: string;   // WebView only, injects JavaScript
}
```

With `testId`, `state` picks the predicate (default `"present"`: the node exists in the tree at
all):

- `"gone"`: no node with this testID exists. A testID that never existed satisfies `"gone"`
  immediately.
- `"visible"`, `"enabled"`, `"focused"`: the node exists and the matching a11y field is `true`.
- `"disabled"`: the node exists and `enabled` is `false`.

`countAtLeast` and `valueEquals`/`valueContains` refine the match:

- `countAtLeast` counts tree nodes whose testID equals `testId`. For any `state` other than
  `"present"`, only actionable ones count, the same rule `resolve` uses. Most testIDs are unique per
  node, so this is usually a 0-or-1 check; it matters when an app reuses one testID across N
  rendered items.
- `valueEquals` and `valueContains` compare against the node's a11y `value` rendered as a string
  (numbers stringified, booleans `"true"`/`"false"`), so one predicate works for a `TextInput` and a
  `Slider` alike. For `TextInput` and `TextArea`, `value` mirrors `text`, making `valueContains` a
  testID-scoped alternative to a global `textContains` search.

### Page predicates

`urlContains`, `pageTitleContains` and `pageTextContains` refine a `testId` selector the same way
`valueContains` does — they are not selectors of their own, so the exactly-one-selector rule is
unchanged. The node the testID names must be a `WebView`; a testID that has not mounted its view yet
simply does not match, so a `waitFor` started before the view exists keeps polling rather than
erroring.

- `urlContains` and `pageTitleContains` read the engine directly (WebKitGTK's `uri`/`title`,
  WKWebView's `url`/`title`). No page JavaScript, no app cooperation.
- `pageTextContains` **injects JavaScript**: `document.body.innerText`, evaluated in the page's own
  world. The result is cached per view and re-probed at most once per 250ms, so a ~50ms tick does
  not run the page fifty times a second — and a match can therefore lag the page by one probe. Do
  not use it against a page where running script is itself the thing under test.

```ts
await client.call("waitFor", { condition: { testId: "tab-webview", urlContains: "wikipedia.org" } });
await client.call("waitFor", { condition: { testId: "tab-webview", pageTextContains: "Result 1" }, timeoutMs: 10000 });
```

`WaitForResult` is `{matched, ref, count}`. `ref` is the winning match, ranked like `resolve`
(actionable first, key window first, then tree order), so the caller needs no follow-up `getTree`.
`count` is how many nodes satisfied the predicate. A timeout is a JSON-RPC error (`-32002`) rather
than a `matched: false` result; see [Error codes](#error-codes).

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

Three behaviors that surprise people:

- A lone `pointer` `down` on a tracking control (slider, button) enters that control's
  mouse-tracking loop until an `up` event arrives. Prefer `drag` for press-move-release sequences.
- `rightClick` auto-appends an escape key press. A context menu's tracking mode does not service the
  main dispatch queue, so a menu left open would wedge every later automation call. The menu still
  opens and closes for real and its hooks run; menu contents are not in the tree yet.
- Coordinate clicks land wherever a real click would. A leading-aligned control whose frame is
  stretched by its container, say a checkbox in a full-width column, only reacts over its visible
  glyph and label region on macOS. Aim at the leading edge of `geometry` rather than the center, or
  use semantic `click`/`setValue`, which do not depend on coordinates.
- A `drag` whose press point lands on a `<slider>` is answered semantically on macOS: the drag's end
  point is mapped to a value and written through the same path `setValue` takes, firing
  `valueChanged` exactly as a user drag does. A synthesized drag cannot move the control otherwise.
  `<slider>` is SwiftUI's `Slider` in an `NSHostingView`, and SwiftUI reads the live pointer state
  rather than the event stream: the posted `mouseDown` lands and the knob jumps to it, then every
  posted `leftMouseDragged` is ignored, with real deltas, with a shared event number, batched or one
  per run-loop turn alike. Pressing the physical button needs `CGEvent.post`, which is the
  accessibility TCC prompt in-process synthesis exists to avoid. Every AppKit control that runs a
  real tracking loop (split-view dividers, table headers) still takes the posted-event path.

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
`type`, `scroll`, `focus`): the ref must exist, be visible, be mapped, and have non-degenerate
on-screen bounds relative to the window. The checks mirror what a real user could reach.

## Coordinate space

`coordinateSpace` is always `"logical-window-topleft"`: every `geometry` field in `getTree` is in
logical units (not device pixels), relative to the window's top-left corner.

## Known gaps

- `scroll` only targets `ScrollView`-typed nodes. A `ListView` node can't be scrolled directly;
  scroll its wrapping `ScrollView` if one exists.
- No `TabView` page-switch RPC. A widget on a non-active tab page is `visible: false`, since GTK and
  AppKit both unmap inactive pages, so it fails the actionability check for `click`, `setValue`, and
  the rest whether targeted by `ref` or `testId`. TestID targeting resolves the node; it does not
  make a hidden one actionable. Put automation-driven controls on the default-active first tab, or
  outside the `TabView`, until this lands.
- No `ListView` row-activate/select action from the RPC side (the widget emits `onRowActivated`
  upward to React, but there's no automation method to trigger it).
- A post-scroll `screenshot` can occasionally race frame invalidation and return a stale texture.
  Retry (poll every ~150ms, up to ~3s) rather than treating one blank shot as final.
- An empty `TextArea` collapses to 0 logical height, so it fails the actionability check until it
  has content or explicit sizing.
- Prefer `setValue({ref, value: boolean})` over `click` for `Checkbox`/`Radio`: `click` toggles the
  current state (relative), while `setValue` sets an exact, deterministic state.
- **GTK cannot reliably rasterize live WebKit content into a screenshot.** The render-to-texture
  path fails whenever WebKit hands the compositor a texture the snapshot renderer cannot download:
  headless cairo fails outright, GL works only for a webview sitting directly in the window, and
  DMABUF on a real GPU fails for both. When the pixel-true pass does work (WebKit rendering in
  software or SHM mode, common under headless weston once a page has settled), the screenshot
  carries real web content. When it fails and webviews are in the window, the snapshot degrades
  rather than erroring: each `<webview>` region is painted as a flat gray plate with a centered
  "WebView" label, the rest of the window renders normally, and the host prints
  `ND_SNAPSHOT_DEGRADED webviews=N` on stderr. That marker is the machine-readable signal, since the
  frozen ABI's `snapshot` op returns only a bool. Windows containing webviews stay capturable; treat
  a degraded capture as layout-true but not pixel-true for the web content.

## Crash/overlay contract

After a runtime crash or disconnect, the host paints an in-window overlay and registers its chrome
widgets in the tree under a reserved generation (`0xFF`), so `getTree` keeps answering through the
crash. Because a JS crash is one Bun process dying, the overlay is painted on every open window.
The registered testIDs are `nd-overlay-panel`, `nd-overlay-title`, `nd-overlay-error`, and
`nd-overlay-restart` (the Restart button); agents can `waitFor` them and drive recovery. Both
backends implement it (`src/gtk/overlay.zig`, `swift/Sources/NDShell/Overlay.swift`).

## Scripted native dialogs

Native file pickers, save panels, and alerts (`NSOpenPanel`/`GtkFileDialog`,
`NSAlert`/`AdwAlertDialog`) are real modal OS UI that an automated run cannot click through. Setting
`ND_AUTOMATION_DIALOG_SCRIPT`, honored only when `NATIVE_AUTOMATION=1`, answers them from a
per-method FIFO instead of opening the real dialog:

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

- **App-level** (`dialog.openFile`, `dialog.saveFile`, `dialog.showMessage`, from `system.ts`'s
  `dialog` object, a `systemRequest`). Each FIFO entry is the raw value the JS call resolves to:
  `string[]` for `dialog.openFile` (chosen paths, `[]` for canceled), `string | null` for
  `dialog.saveFile`, the clicked button's index for `dialog.showMessage`.
- **Window-scoped** (`window.showAlert`, `window.openFile`, `window.saveFile`, from `dialogs.ts`, a
  `widgetCommand` on a `<window>` node). Each entry is the result shape the promise resolves to:
  `AlertResult {buttonId}`, `OpenFileResult {canceled, paths}`, `SaveFileResult {canceled, path}`.

Each call shifts the head of its method's queue. An exhausted queue never falls through to the real
dialog; it fails loudly. App-level exhaustion rejects the pending `systemRequest` with `ok: false`
and `"dialog script exhausted: <method>"`. Window-scoped exhaustion has no promise to reject, since
there is no `*Result` event to synthesize, so it prints `ND_DIALOG_SCRIPT_EXHAUSTED method=<name>`
on stderr. Poll for that marker with `AppHandle.waitForMarker` (see
[Test Harness](/automation-testing/test-harness/)) to assert the drain.

## Screenshots on macOS (ndshot)

The `screenshot` RPC renders offscreen inside the host process, and on macOS 26 that path draws
blank editable fields for `TextInput` and `TextArea`, because `_NSCoreHostingView` only paints via
CoreAnimation when composited on screen. `tools/ndshot/` works around it: a small dependency-free
Swift package with its own stable binary identity that requests Screen Recording once, then captures
the live composited window via ScreenCaptureKit. Capture works even when the window is occluded.

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

Freshness contract: ScreenCaptureKit samples the window server's composite rather than the live view
tree, so a commit AppKit has applied but never displayed captures stale. Measured on the panes
example, a status label reached the composite while a freshly inserted `NSSplitView` subtree stayed
blank until the RPC ladder's own `displayIfNeeded` healed it. Hosts running with
`NATIVE_AUTOMATION=1` therefore run a ~100ms main-runloop tick that forces
`layoutSubtreeIfNeeded`/`displayIfNeeded` on every visible window plus a `CATransaction.flush`, and
the in-process SCK rung (`ND_AUTOMATION_CAPTURE=screencapturekit`) flushes the same way immediately
before capturing. An external `ndshot` capture of an automation host is current as of the last tick,
well inside its own ~250ms frame-stability resample. Capturing a non-automation app has no such
guarantee.
