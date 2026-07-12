---
title: MCP Tools
description: packages/mcp bridges the automation socket to four MCP tools — a thin pass-through, not a reinterpretation.
---

`packages/mcp` is a stdio MCP server that bridges an agent's tool calls to the host's
[automation socket](/automation-testing/automation-socket/). It exposes four tools today, each a
thin pass-through to the raw RPC method of the same name:

| MCP tool | RPC method | Notes |
|---|---|---|
| `nd_get_tree` | `getTree` | snapshot the widget tree (refs, `testID`s, text, geometry) |
| `nd_screenshot({path})` | `screenshot` | render the window to a PNG at an absolute path |
| `nd_click({ref})` | `click` | semantic click on a widget by ref |
| `nd_wait_for({textContains?, refVisible?, timeoutMs?})` | `waitFor` | poll a tree condition until it holds or times out |

## The raw socket has more

`setValue`, `type`, and `scroll` exist on the automation socket but don't have MCP tool wrappers
yet. Drive them by talking to the socket directly — `packages/mcp/src/socket.ts`'s
`AutomationClient` is the client-side pattern every `scripts/*-drive.ts` script in this repo uses,
and is a reasonable template for a custom driver:

```ts
import { AutomationClient } from "../packages/mcp/src/socket.ts"; // path relative to your script

const client = await AutomationClient.connect(); // reads ND_AUTOMATION_SOCKET, or pass a path
await client.call("setValue", { ref, value: true });
```

See [Automation Socket](/automation-testing/automation-socket/) for the full method list, error
codes, and known gaps (actionability rules, `Checkbox`/`Radio` semantics, scroll targeting).

## Crash debugging for agents

**Not yet landed.** The plan is for `getTree` to expose `nd-overlay-title`/`nd-overlay-error`/
`nd-overlay-restart` testIDs once the crash-overlay lands, so an agent can read a crash the same way
it reads any other tree state and, in dev mode, click `nd-overlay-restart` to respawn the child.
Today, a crash simply prints `ND_CHILD_EXITED` and the window goes stale with no tracked recovery
path — treat any crash as fatal to the current run until this lands.
