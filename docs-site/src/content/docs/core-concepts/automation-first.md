---
title: Automation-First
description: Every widget is inspectable and drivable over a JSON-RPC socket, so coding agents are a first-class user.
---

Every widget the React tree creates is tracked host-side and answerable over a socket, so an agent
or a headless CI script inspects and drives an app the way a person does.

## The RPC surface

Every method's params, result shape, and error code is generated from `schema/rpc.json`, shared by
the Zig host (`src/generated/rpc.zig`, consumed by `src/automation.zig`) and the TypeScript client
(`packages/react/src/generated/rpc.ts`). Rename or retype a field there and both sides regenerate,
making a mismatch a compile error rather than a silent wire break. Same `tools/codegen.ts` pipeline
that generates widget bindings from `schema/widgets.json`.

The host exposes a framed JSON-RPC 2.0 socket, gated on `NATIVE_AUTOMATION=1`. Seven methods are
semantic and work on both backends:

| Method | What it does |
|---|---|
| `getTree` | Accessibility snapshot of the widget tree: refs, `testID`s, text, visibility, geometry, role, enabled, focused, value. |
| `screenshot` | Render a window to a PNG. |
| `click` | Semantic click on a widget by ref, actionability-checked. |
| `waitFor` | Poll a tree condition (`textContains` or `refVisible`) until it holds or times out. |
| `setValue` | Kind-dispatched value set: bool for `Checkbox` and `Radio`, number for `Slider`, string for `TextInput` and `TextArea`, index for `Select`. |
| `type` | Semantic text append on a `TextInput`, through `GtkEditable.insertText` rather than synthetic keysyms. |
| `scroll` | Adjust a `ScrollView`'s scroll position. |

Six more synthesize real input (`pointer`, `drag`, `keys`, `doubleClick`, `rightClick`, `hover`) by
posting `NSEvent`s through the app's own queue. Those are macOS-only: GTK4 removed
app-constructible events, so on Linux they answer `-32003` and you use the semantic methods instead.

These plus `resolve`, `windows`, `focus`, `scrollIntoView`, `snapshotNode`, `setWindowFrame`,
and the webview-only `webviewInfo`/`webviewEval` round out the socket; method-by-method detail,
including error codes, lives on the [Automation Socket](/automation-testing/automation-socket/) page.

Driving a widget directly against this socket means resolving a `ref` or a `testId` yourself and
retrying past timing races by hand. `@nativedesktop/test`'s [locators](/automation-testing/locators/)
(`app.getByRole/getByTestId/getByText(...).click()`, `expect(locator).toBeVisible()`) are layered on
top of it and are the recommended surface for anything that targets a widget: they re-resolve on
every action and run an actionability ladder (visible, enabled, real geometry, a stable frame) before
dispatching. The RPCs on this page are what they compile down to, and are still the right level for
host-side polling (`waitFor` and its sugar) and anything a locator does not wrap, like a table row's
own sub-region.

## `ND_*` markers

Every host prints a stable set of markers to stderr: `ND_CHILD_CONNECTED`,
`ND_COMMIT_APPLIED commitId=…`, `ND_AUTOMATION_LISTENING path=…`, `ND_CHILD_EXITED`, and others. A
drive script or an agent waits for a marker instead of parsing arbitrary host output.

## The drive-script pattern

Every `scripts/*-drive.ts` in this repo starts the same way: launch the host with
`NATIVE_AUTOMATION=1`, wait for `ND_AUTOMATION_LISTENING` on stderr to learn the socket path, and
connect (`connectApp()` when the gate's own bash owns the process, `launchApp()` otherwise). Most
then drive the app through `@nativedesktop/test`'s [locators](/automation-testing/locators/) and
`expect`; a handful still issue raw `getTree`/`click`/`setValue`/`waitFor` calls against the
`AutomationClient` directly (`packages/test/src/socket.ts`), which is still how the RPCs on this page
get exercised. `scripts/notes-drive.ts` and the HMR leg of `scripts/headless-m8.sh` are worked
examples of the raw pattern: click to a known state, edit a live source file, then assert the UI
reflects it without losing state or disconnecting.

## Actionability checks

`click`, `setValue`, `type`, and `scroll` are actionability-checked first. The ref must exist, be
visible, be mapped, and have non-degenerate on-screen bounds relative to the window, mirroring what
a real user could reach. A failed check returns error `-32001` with a reason (`unknown`,
`invisible`, `unmapped`, or `offscreen`) instead of no-opping quietly.

See [Automation Socket](/automation-testing/automation-socket/) for the full transport and method
reference, and [MCP Tools](/automation-testing/mcp-tools/) for the higher-level tool wrappers an
agent typically calls instead of the raw socket.
