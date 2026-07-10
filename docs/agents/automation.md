# Automation — the full RPC surface

Ground truth for everything below is `src/automation.zig`; this doc is a human/agent-readable
mirror of it, not an independent spec — if the two disagree, the Zig source wins.

## Transport

Framed JSON-RPC 2.0: a u32 little-endian length prefix followed by the UTF-8 JSON payload, one
frame per message, over a unix domain socket at `$XDG_RUNTIME_DIR/nd-automation-<host-pid>.sock`.
The host prints the socket path as `ND_AUTOMATION_LISTENING path=<path>` on stderr once it's ready.
The server is gated on `NATIVE_AUTOMATION=1` in the host's environment (unset = the socket never
opens). `packages/mcp` is a stdio MCP server that bridges this socket to MCP tool calls (see below).

## Methods

| Method | Params | Result | Notes |
|---|---|---|---|
| `getTree` | none | `{coordinateSpace: "logical-window-topleft", root: JsonNode}` | full tree snapshot |
| `screenshot` | `{path: string, window?: number}` | `{path, width, height}` | in-process render → PNG; `window` (if given) must match the root ref |
| `click` | `{ref: number}` | `{ref, dispatched: true}` | actionability-checked, emits `clicked` semantically |
| `waitFor` | `{condition: {textContains?: string} \| {refVisible?: number}, timeoutMs?: number}` | `{matched: true}` | polls at ~50ms; default `timeoutMs` 2000 |
| `setValue` | `{ref: number, value: string \| boolean \| number}` | `{ref, applied: true}` | kind-dispatched: `TextInput`/`TextArea` need a string, `Checkbox`/`Radio` a bool, `Slider` a number, `Select` an integer index |
| `type` | `{ref: number, text: string}` | `{ref, text: <full text after insert>}` | `TextInput` only; semantic append via `GtkEditable.insertText`, never synthetic keysyms |
| `scroll` | `{ref: number, dx?: number, dy?: number}` | `{ref, x, y}` (resulting adjustment values) | `ScrollView` only — see Deltas below |

`JsonNode` shape (from `getTree`/nested in `root`/`children`): `{ref: number, type: string, testID:
string \| null, text: string \| null, visible: boolean, geometry: {x,y,w,h} \| null, children:
JsonNode[], itemCount: number \| null}`. `itemCount` is non-null only for data-driven widgets
(currently `ListView`); it is the row count, never a walk of GTK's recycled row widgets.

## Error codes

| Code | Meaning | `data` shape |
|---|---|---|
| `-32001` | not actionable | `{ref, reason: "unknown" \| "invisible" \| "unmapped" \| "offscreen"}` |
| `-32002` | `waitFor` timed out | `{timeoutMs}` |
| `-32601` | method not found | none |
| `-32602` | invalid params | `{ref}` where applicable (e.g. missing/wrong-typed value, unsupported widget kind for `setValue`/`type`/`scroll`) |
| `-32603` | internal error | none (or a message describing the failure, e.g. screenshot renderer/surface errors) |
| `-32700` | parse error | none |

Actionability (`-32001`) is checked before every action-dispatch method (`click`, `setValue`,
`type`, `scroll`): the ref must exist, be visible, be mapped, and have non-degenerate on-screen
bounds relative to the window. This mirrors what a real user could reach — automation never acts on
what a user couldn't.

## Coordinate-space contract

`coordinateSpace` is always `"logical-window-topleft"`: every `geometry` field in `getTree` is in
logical units (not device pixels), relative to the window's top-left corner — computed via
`gtk.Widget.computeBounds(widget, window_widget, &rect)`.

## MCP tool names

`packages/mcp/src/index.ts` exposes four tools today, each a thin pass-through to the raw methods
above:

- `nd_get_tree` → `getTree`
- `nd_screenshot({path})` → `screenshot`
- `nd_click({ref})` → `click`
- `nd_wait_for({textContains?, refVisible?, timeoutMs?})` → `waitFor`

`setValue`/`type`/`scroll` exist on the raw socket but do not yet have MCP tool wrappers — drive
them by talking to the automation socket directly (see `packages/mcp/src/socket.ts`'s
`AutomationClient` for the client-side pattern, used by every `scripts/*-drive.ts` script).

## Deltas (known gaps — do not assume these work)

- **`scroll` only targets `ScrollView`-typed nodes** (their wrapping `GtkScrolledWindow`
  adjustments). A `ListView` node cannot be scrolled directly — scroll its wrapping `ScrollView`
  instead, if one wraps it.
- **No `TabView` page-switch RPC.** There is no automation action to change which tab is active.
- **No `ListView` row-activate/select action.** The widget emits `onRowActivated` upward to React,
  but there is no automation method to trigger row activation/selection from the RPC side.
- **Screenshot-after-scroll can race frame invalidation.** Taking a `screenshot` immediately after a
  `scroll` can occasionally return a texture from before the scroll finished compositing
  (`WidgetPaintable` served empty briefly in testing). If a post-scroll screenshot looks stale,
  retry (poll every ~150ms, up to ~3s) rather than treating a single failed/blank shot as final.

## Crash/overlay contract — planned, not yet landed

The plan for the M8 overlay task (see `docs/superpowers/plans/2026-07-10-m8-dx.md`) is: after a
runtime crash or disconnect, the host paints an in-window overlay and registers its widgets in the
tree under a **reserved generation `0xFFFF`**, specifically so `getTree` keeps answering through a
crash instead of going stale. Planned testIDs: `nd-overlay-title`, `nd-overlay-error`,
`nd-overlay-restart` — read `nd-overlay-error`'s `text` for the failure message, and (dev-mode only)
`click` the `nd-overlay-restart` ref to respawn the crashed child. The runtime is planned to report
uncaught exceptions via an additive `runtimeError {message, stack}` NDP control frame before it
dies, so the overlay shows the real error rather than a bare disconnect notice.

**(lands with M8 overlay task)** — none of the above exists in `src/automation.zig` or
`src/runtime.zig` today. A crash today is simply `ND_CHILD_EXITED` on stderr with no tracked
recovery node in the tree; `getTree` after a crash will fail or return stale data, not an overlay
snapshot. Do not write agent logic that assumes `nd-overlay-*` testIDs exist until this section is
updated to say the task has landed.
