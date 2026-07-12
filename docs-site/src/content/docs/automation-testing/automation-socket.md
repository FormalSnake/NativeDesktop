---
title: Automation Socket
description: The full JSON-RPC automation surface — transport, methods, error codes, and known gaps.
---

Ground truth for this page is `src/automation.zig`; this is a human/agent-readable mirror of it, not
an independent spec — if the two disagree, the Zig source wins.

## Transport

Framed JSON-RPC 2.0: a `u32` little-endian length prefix followed by the UTF-8 JSON payload, one
frame per message, over a unix domain socket at `$XDG_RUNTIME_DIR/nd-automation-<host-pid>.sock`.
The host prints the socket path as `ND_AUTOMATION_LISTENING path=<path>` on stderr once it's ready.
The server is gated on `NATIVE_AUTOMATION=1` in the host's environment — unset, the socket never
opens. `packages/mcp` is a stdio MCP server that bridges this socket to MCP tool calls — see
[MCP Tools](/automation-testing/mcp-tools/).

## Methods

| Method | Params | Result | Notes |
|---|---|---|---|
| `getTree` | none | `{coordinateSpace, root: JsonNode}` | full tree snapshot |
| `screenshot` | `{path, window?}` | `{path, width, height}` | in-process render → PNG; `window` (if given) must match the root ref |
| `click` | `{ref}` | `{ref, dispatched: true}` | actionability-checked, emits `clicked` semantically |
| `waitFor` | `{condition: {textContains?} \| {refVisible?}, timeoutMs?}` | `{matched: true}` | polls at ~50ms; default `timeoutMs` 2000 |
| `setValue` | `{ref, value}` | `{ref, applied: true}` | kind-dispatched: `TextInput`/`TextArea` need a string, `Checkbox`/`Radio` a bool, `Slider` a number, `Select` an integer index |
| `type` | `{ref, text}` | `{ref, text: <full text after insert>}` | `TextInput` only; semantic append via `GtkEditable.insertText`, never synthetic keysyms |
| `scroll` | `{ref, dx?, dy?}` | `{ref, x, y}` (resulting adjustment values) | `ScrollView` only |

`JsonNode` (from `getTree`, nested under `root`/`children`):
`{ref, type, testID, text, visible, geometry: {x,y,w,h} | null, children, itemCount}`. `itemCount`
is non-null only for data-driven widgets (currently `ListView`) — the row count, never a walk of
recycled row widgets.

## Error codes

| Code | Meaning | `data` shape |
|---|---|---|
| `-32001` | not actionable | `{ref, reason: "unknown" \| "invisible" \| "unmapped" \| "offscreen"}` |
| `-32002` | `waitFor` timed out | `{timeoutMs}` |
| `-32601` | method not found | none |
| `-32602` | invalid params | `{ref}` where applicable |
| `-32603` | internal error | none, or a message describing the failure |
| `-32700` | parse error | none |

Actionability (`-32001`) is checked before every action-dispatch method (`click`, `setValue`,
`type`, `scroll`): the ref must exist, be visible, be mapped, and have non-degenerate on-screen
bounds relative to the window — mirroring what a real user could reach.

## Coordinate space

`coordinateSpace` is always `"logical-window-topleft"`: every `geometry` field in `getTree` is in
logical units (not device pixels), relative to the window's top-left corner.

## Known gaps — do not assume these work

- `scroll` only targets `ScrollView`-typed nodes. A `ListView` node can't be scrolled directly —
  scroll its wrapping `ScrollView` if one exists.
- No `TabView` page-switch RPC.
- No `ListView` row-activate/select action from the RPC side (the widget emits `onRowActivated`
  upward to React, but there's no automation method to trigger it).
- A post-scroll `screenshot` can occasionally race frame invalidation and return a stale texture —
  retry (poll every ~150ms, up to ~3s) rather than treating one blank shot as final.
- An empty `TextArea` collapses to 0 logical height, failing the actionability check until it has
  content or explicit sizing.
- Prefer `setValue({ref, value: boolean})` over `click` for `Checkbox`/`Radio` — `click` toggles the
  current state (relative), while `setValue` sets an exact, deterministic state.

## Crash/overlay contract — planned, not yet landed

The plan: after a runtime crash or disconnect, the host paints an in-window overlay and registers
its widgets under a reserved generation (`0xFFFF`) so `getTree` keeps answering through a crash.
Planned testIDs: `nd-overlay-title`, `nd-overlay-error`, `nd-overlay-restart`. **None of this exists
today** — a crash currently prints `ND_CHILD_EXITED` with no tracked recovery node, and `getTree`
after a crash fails or returns stale data. Do not write agent logic that assumes `nd-overlay-*`
testIDs exist yet.

## Screenshots on macOS (ndshot)

The `screenshot` RPC renders offscreen inside the host process, and on macOS 26 that path draws
blank editable fields for `TextInput`/`TextArea` (`_NSCoreHostingView` only paints via CoreAnimation
when actually composited on screen). `tools/ndshot/` works around this: a small, dependency-free
Swift package with its own stable binary identity that preflights/requests Screen Recording once,
then captures the *live composited* window via ScreenCaptureKit — this works even when the window is
occluded.

```bash
cd tools/ndshot && ./build.sh
```

Three subcommands under `.build/release/ndshot`: `doctor` (reports Screen Recording permission
state; exit 0 granted, 2 not), `list` (enumerates capturable windows as JSON lines), and
`capture --out <path.png> [--pid <pid>] [--title <substring>] [--window-id <id>]`. The first
invocation of `list`/`capture` triggers the system's one-time, headful Screen Recording permission
prompt — grant it once via System Settings → Privacy & Security → Screen Recording; the grant then
sticks to this binary's path and ad hoc signature across future runs and rebuilds (a rebuild that
changes the binary's bytes counts as a new identity and needs re-granting).
