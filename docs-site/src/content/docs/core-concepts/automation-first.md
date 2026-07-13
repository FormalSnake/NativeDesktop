---
title: Automation-First
description: Every widget is inspectable and drivable over a JSON-RPC socket — coding agents are a first-class user.
---

NativeDesktop's automation layer isn't a testing add-on bolted onto a finished framework — it's a
design constraint from the start: every widget the React tree creates is tracked host-side and
answerable over a socket, so an agent (or a headless CI script) can inspect and drive an app the
same way a human would.

## The RPC surface, in brief

Every method's params/result shape and every error code is generated from `schema/rpc.json` — the
single source of truth shared by the Zig host (`src/generated/rpc.zig`, consumed by
`src/automation.zig`) and the TS client (`packages/react/src/generated/rpc.ts`). Renaming or
retyping a field there regenerates both sides, so a Zig↔Bun mismatch is a compile error, not a
silent wire break — the same `tools/codegen.ts` pipeline that generates widget bindings from
`schema/widgets.json`. The host exposes this as a framed JSON-RPC 2.0 socket, gated on
`NATIVE_AUTOMATION=1`:

- `getTree` — a full snapshot of the widget tree: refs, `testID`s, text, visibility, geometry.
- `screenshot` — render the window to a PNG.
- `click` — semantic click on a widget by ref (actionability-checked).
- `waitFor` — poll a tree condition (`textContains` or `refVisible`) until it holds or times out.
- `setValue` — kind-dispatched value set (`Checkbox`/`Radio` take a bool, `Slider` a number,
  `TextInput`/`TextArea` a string, `Select` an index).
- `type` — semantic text append on a `TextInput`, via `GtkEditable.insertText`, never synthetic
  keysyms.
- `scroll` — adjust a `ScrollView`'s scroll position.

Full method-by-method detail, including error codes, lives on the
[Automation Socket](/automation-testing/automation-socket/) page.

## `ND_*` markers: what to grep for

Every host prints a stable vocabulary of markers to stderr — `ND_CHILD_CONNECTED`,
`ND_COMMIT_APPLIED commitId=…`, `ND_AUTOMATION_LISTENING path=…`, `ND_CHILD_EXITED`, and more. A
drive script (or an agent) doesn't need to parse arbitrary host output; it waits for a marker.

## The drive-script pattern

Every `scripts/*-drive.ts` script in this repo follows the same shape: launch the host with
`NATIVE_AUTOMATION=1`, wait for `ND_AUTOMATION_LISTENING` on stderr to learn the socket path,
connect an `AutomationClient` (`packages/mcp/src/socket.ts`) to it, then issue a sequence of
`getTree`/`click`/`setValue`/`waitFor` calls asserting on the results — exactly the loop a coding
agent runs interactively, just scripted. `scripts/notes-drive.ts` and `scripts/headless-m8.sh`'s HMR
leg are real examples: click to a known state, edit a live source file, and assert the UI reflects
it without losing state or disconnecting.

## Actionability is real, not best-effort

Every action-dispatch method (`click`, `setValue`, `type`, `scroll`) is actionability-checked first:
the ref must exist, be visible, be mapped, and have non-degenerate on-screen bounds relative to the
window — mirroring what a real user could actually reach. Automation never acts on what a user
couldn't; a failed actionability check returns error `-32001` with a reason (`unknown` / `invisible`
/ `unmapped` / `offscreen`), not a silent no-op.

See [Automation Socket](/automation-testing/automation-socket/) for the full transport and method
reference, and [MCP Tools](/automation-testing/mcp-tools/) for the higher-level tool wrappers an
agent typically calls instead of the raw socket.
