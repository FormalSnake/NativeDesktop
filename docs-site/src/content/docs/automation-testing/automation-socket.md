---
title: Automation Socket
description: The full JSON-RPC automation surface, covering transport, methods, error codes, and known gaps.
---

Ground truth for this page is `schema/rpc.json`, the single source of truth `tools/codegen.ts`
generates both `src/generated/rpc.zig` (consumed by `src/automation.zig`) and
`packages/react/src/generated/rpc.ts` from. This page is a human/agent-readable mirror of the
schema; if the two disagree, the schema (and its generated output) wins.
A method/param/result change in the schema is a compile error on both the Zig and TypeScript side
until every caller is updated, not a silent runtime break.

## Transport

Framed JSON-RPC 2.0: a `u32` little-endian length prefix followed by the UTF-8 JSON payload, one
frame per message, over a unix domain socket at `$XDG_RUNTIME_DIR/nd-automation-<host-pid>.sock`.
The host prints the socket path as `ND_AUTOMATION_LISTENING path=<path>` on stderr once it's ready.
The server is gated on `NATIVE_AUTOMATION=1` in the host's environment; if it's unset, the socket
never opens. `packages/mcp` is a stdio MCP server that bridges this socket to MCP tool calls; see
[MCP Tools](/automation-testing/mcp-tools/).

## Methods

| Method | Params | Result | Notes |
|---|---|---|---|
| `getTree` | `{window?}` | `{coordinateSpace, root: JsonNode}` | accessibility-tree snapshot; `window` (a Window node ref) scopes it to that window's subtree |
| `screenshot` | `{path, window?}` | `{path, width, height}` | in-process render → PNG; `window` picks any open window's Window node ref |
| `click` | `{ref}` | `{ref, dispatched: true}` | actionability-checked, emits `clicked` semantically |
| `waitFor` | `{condition: {textContains?} \| {refVisible?}, timeoutMs?}` | `{matched: true}` | polls at ~50ms; default `timeoutMs` 2000 |
| `setValue` | `{ref, value}` | `{ref, applied: true}` | kind-dispatched: `TextInput`/`TextArea` need a string, `Checkbox`/`Radio` a bool, `Slider` a number, `Select` an integer index |
| `type` | `{ref, text}` | `{ref, text: <full text after insert>}` | `TextInput` only; semantic append via `GtkEditable.insertText`, never synthetic keysyms |
| `scroll` | `{ref, dx?, dy?}` | `{ref, x, y}` (resulting adjustment values) | `ScrollView` only |
| `doubleClick` | `{ref}` | `{ref, dispatched: true}` | real double-click at the widget's center (activates table/list rows); macOS only |
| `rightClick` | `{ref}` | `{ref, dispatched: true}` | real right-click at the widget's center; an opened context menu is auto-dismissed with an escape (see below); macOS only |
| `hover` | `{ref}` | `{ref, dispatched: true}` | best-effort `mouseMoved` at the widget's center; macOS only |
| `pointer` | `{phase: "down"\|"move"\|"up", x, y, button?, clickCount?, window?}` | `{dispatched: true}` | low-level single pointer phase at window-topleft coordinates; `clickCount: 2` on a down/up pair makes a double-click; macOS only |
| `drag` | `{fromRef?\|fromX?,fromY?, toRef?\|toX?,toY?, steps?, durationMs?, button?, window?}` | `{dispatched, fromX, fromY, toX, toY, steps}` | press-move-release; ref endpoints resolve to widget centers and must share a window; macOS only |
| `keys` | `{keys, window?}` | `{dispatched: true}` | `"cmd+shift+n"` presses one chord (drives menu key equivalents); `"escape"`/`"tab"` a named key; any other string types its characters into the focused widget; macOS only |

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

## Input synthesis — platform support

`pointer`/`drag`/`keys`/`doubleClick`/`rightClick`/`hover` post real native events on macOS:
constructed `NSEvent`s pushed through the app's own event queue (`NSApp.postEvent`), so slider
thumbs, split-view dividers, table row double-activation, text selection, and menu key equivalents
run genuine AppKit machinery, in-process and without any TCC permission. Multi-event gestures (`drag`,
double-clicks) are posted as one batch because AppKit controls run nested mouse-tracking loops
inside `mouseDown` dispatch that consume the rest of the gesture from the queue while blocking the
main thread.

On GTK these methods answer `-32003` (`input synthesis unsupported on this backend`): GTK4 removed
app-constructible `GdkEvent`s, so in-process synthesis is impossible. Use the semantic methods
(`click`/`setValue`/`type`/`scroll`) on Linux instead. The `a11y` fields work on both backends.

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
| `-32003` | input synthesis unsupported on this backend | `{ref}` — pointer/drag/keys/doubleClick/rightClick/hover on GTK |
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
- No `TabView` page-switch RPC.
- No `ListView` row-activate/select action from the RPC side (the widget emits `onRowActivated`
  upward to React, but there's no automation method to trigger it).
- A post-scroll `screenshot` can occasionally race frame invalidation and return a stale texture.
  Retry (poll every ~150ms, up to ~3s) rather than treating one blank shot as final.
- An empty `TextArea` collapses to 0 logical height, so it fails the actionability check until it
  has content or explicit sizing.
- Prefer `setValue({ref, value: boolean})` over `click` for `Checkbox`/`Radio`: `click` toggles the
  current state (relative), while `setValue` sets an exact, deterministic state.

## Crash/overlay contract

After a runtime crash or disconnect, the host paints an in-window overlay and registers its chrome
widgets in the tree under a reserved generation (`0xFF`), so `getTree` keeps answering through the
crash. Because a JS crash is one Bun process dying, the overlay is painted on every open window.
The registered testIDs are `nd-overlay-panel`, `nd-overlay-title`, `nd-overlay-error`, and
`nd-overlay-restart` (the Restart button); agents can `waitFor` them and drive recovery. Both
backends implement it (`src/gtk/overlay.zig`, `swift/Sources/NDShell/Overlay.swift`).

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
