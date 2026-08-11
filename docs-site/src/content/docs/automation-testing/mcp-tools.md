---
title: MCP Tools
description: packages/mcp bridges the automation socket to MCP tools, each a thin pass-through to the raw RPC method of the same name.
---

`packages/mcp` is a stdio MCP server that bridges an agent's tool calls to the host's
[automation socket](/automation-testing/automation-socket/). Every tool is a thin pass-through to
the raw RPC method of the same name:

| MCP tool | RPC method | Notes |
|---|---|---|
| `nd_get_tree({window?})` | `getTree` | accessibility-tree snapshot (refs, `testID`s, text, geometry, role, enabled, focused, value); `window` scopes to one window |
| `nd_screenshot({path})` | `screenshot` | render the window to a PNG at an absolute path |
| `nd_click({ref})` | `click` | semantic click on a widget by ref |
| `nd_wait_for({textContains?, refVisible?, timeoutMs?})` | `waitFor` | poll a tree condition until it holds or times out |
| `nd_set_value({ref, value})` | `setValue` | set a widget's value semantically; fires the native change event |
| `nd_type({ref, text})` | `type` | semantic text append into a `TextInput` |
| `nd_scroll({ref, dx?, dy?})` | `scroll` | scroll a `ScrollView` by logical units |
| `nd_double_click({ref})` | `doubleClick` | real double-click at the widget's center (macOS only) |
| `nd_right_click({ref})` | `rightClick` | real right-click, auto-dismissing any opened context menu (macOS only) |
| `nd_hover({ref})` | `hover` | best-effort pointer hover at the widget's center (macOS only) |
| `nd_pointer({phase, x, y, button?, clickCount?, window?})` | `pointer` | low-level pointer phase at window coordinates (macOS only) |
| `nd_drag({fromRef?/toRef? or coordinates, steps?, durationMs?, window?})` | `drag` | press-move-release gesture for slider thumbs, dividers, and selections (macOS only) |
| `nd_keys({keys, window?})` | `keys` | chord like `"cmd+n"` (drives menu key equivalents) or plain text typed into the focused widget (macOS only) |

The macOS-only tools post real `NSEvent`s through the app's event queue; on GTK they answer
`-32003` (`input synthesis unsupported on this backend`). See the
[platform support notes](/automation-testing/automation-socket/#input-synthesis--platform-support).

## Talking to the socket directly

`@nativedesktop/test`'s `AutomationClient` (`packages/test/src/socket.ts`) is the client-side pattern every
`scripts/*-drive.ts` script in this repo uses, and is a reasonable template for a custom driver.
`AutomationClient.call` is generic over the method names generated from `schema/rpc.json`, so the
method name, its params, and its result type are all checked at compile time; a typo or a stale
param shape is a `tsc` error, not a runtime surprise:

```ts
import { AutomationClient } from "@nativedesktop/test";

const client = await AutomationClient.connect(); // reads ND_AUTOMATION_SOCKET, or pass a path
await client.call("setValue", { ref, value: true });
await client.call("drag", { fromRef: sliderRef, toX: 400, toY: 120 });
```

See [Automation Socket](/automation-testing/automation-socket/) for the full method list, error
codes, and known gaps (actionability rules, `Checkbox`/`Radio` semantics, scroll targeting).

## Crash debugging for agents

After a runtime crash or disconnect, the host paints an in-window overlay on every open window and
registers its chrome in the tree, so `getTree` keeps answering through the crash. Agents can
`nd_wait_for` the overlay testIDs (`nd-overlay-panel`, `nd-overlay-title`, `nd-overlay-error`,
`nd-overlay-restart`), read the error text off the tree, and in dev mode (`ND_DEV=1`) click
`nd-overlay-restart` to respawn the child. See the
[crash/overlay contract](/automation-testing/automation-socket/#crashoverlay-contract).
