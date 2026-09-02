---
title: MCP Tools
description: "packages/mcp is a stdio MCP server with three tools: execute runs @nativedesktop/test code against a live app, snapshot reads the accessibility tree, reset relaunches the host."
---

`packages/mcp` is a stdio MCP server that hands an agent a live NativeDesktop app. There are three
tools, not one per RPC: an agent writes `@nativedesktop/test` code, so anything the harness can
express is reachable without a new tool being added for it.

```
ND_MCP_ENTRY=examples/counter/main.tsx bun packages/mcp/src/index.ts
```

## `snapshot({window?, interactiveOnly?})`

The accessibility tree, one line per node, indented by depth. Read it before acting, the way you
would read a page. Nodes with no role, text or testID collapse into the parent, which removes most
of a real tree.

```
- window "NativeDesktop M3 Counter" [ref=e7] [enabled]
  - group [ref=e6] [enabled]
    - label "Clicks: 0" [ref=e1] [testid=clicks-label] [enabled]
    - button "Increment" [ref=e2] [testid=increment-button] [enabled]
```

`ref=eN` is the wire ref every action targets. `window` scopes to one Window node (from
`app.windows()`); `interactiveOnly` keeps only the widgets an action can reach.

## `execute({code, timeout?})`

Runs a snippet with `{app, state, expect, launchApp, snapshot}` in scope. Prefer one line, with `;`
between statements, and call `execute` again rather than writing a script in a single call.

```js
await app.getByTestId("increment-button").click(); await expect(app.getByTestId("clicks-label")).toContainText("Clicks: 1")
```

```json
{ "result": "Clicks: 1", "logs": [] }
```

`app` is the [locator surface](/automation-testing/locators/). The last expression is the result, a
`Locator` serializes to its selector string, a tree node to `{ref, type, role, text, testID}`, and a
cycle to `"[Circular]"`. `console.log` is captured into `logs` (stdout is the MCP transport, so a
stray log would frame the protocol out of sync). `state` is a plain object that survives across
calls, for stashing a ref or a window between one-liners.

A rejected RPC comes back with its code and data intact:

```json
{ "error": "not actionable (-32001)", "code": -32001, "data": { "ref": 12, "reason": "invisible" } }
```

A dead host answers `{error, stderrTail, hint: "call reset"}`.

## `reset({entry?, backend?})`

Closes the app, launches it again, and clears `state`. Use it after a crash, a wedged dialog, or to
start a scenario from a known screen. Answers the new `{mode, pid, socket, entry}`.

## Attaching instead of spawning

With `ND_AUTOMATION_SOCKET` set, the server attaches to a host somebody else launched, and `reset`
reconnects rather than respawning. Without it, the entry comes from `ND_MCP_ENTRY` (and the backend
from `ND_MCP_BACKEND`), which `reset({entry})` can change at runtime.

## Crash debugging for agents

After a runtime crash or disconnect, the host paints an in-window overlay on every open window and
registers its chrome in the tree, so `getTree` keeps answering through the crash. `snapshot` shows
the overlay testIDs (`nd-overlay-panel`, `nd-overlay-title`, `nd-overlay-error`,
`nd-overlay-restart`), the error text reads off the tree, and in dev mode (`ND_DEV=1`) clicking
`nd-overlay-restart` respawns the child. See the
[crash/overlay contract](/automation-testing/automation-socket/#crashoverlay-contract).

## Talking to the socket directly

`@nativedesktop/test`'s `AutomationClient` (`packages/test/src/socket.ts`) is the raw client, and a
reasonable template for a custom driver. `AutomationClient.call` is generic over the method names
generated from `schema/rpc.json`, so the method name, its params, and its result type are checked at
compile time:

```ts
import { AutomationClient } from "@nativedesktop/test";

const client = await AutomationClient.connect(); // reads ND_AUTOMATION_SOCKET, or pass a path
await client.call("setValue", { ref, value: true });
await client.call("drag", { fromRef: sliderRef, toX: 400, toY: 120 });
```

See [Automation Socket](/automation-testing/automation-socket/) for the full method list, error
codes, and the platform gaps (input synthesis answers `-32003` on GTK).
