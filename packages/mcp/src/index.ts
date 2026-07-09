#!/usr/bin/env bun
// Stdio MCP server wrapping the NativeDesktop automation socket (ND_AUTOMATION_SOCKET).
// Exposes nd_get_tree / nd_screenshot / nd_click / nd_wait_for as agent tools, each a
// thin pass-through to the framed JSON-RPC automation server (see docs/superpowers/plans
// /2026-07-09-m4-automation.md, RPC surface v1).

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import { AutomationClient } from "./socket.ts";

const client = await AutomationClient.connect();
const server = new McpServer({ name: "nativedesktop", version: "0.0.0" });

server.registerTool(
  "nd_get_tree",
  {
    description: "Snapshot the app widget tree with stable refs, testIDs, text, and logical geometry.",
    inputSchema: {},
  },
  async () => ({ content: [{ type: "text" as const, text: JSON.stringify(await client.call("getTree"), null, 2) }] }),
);

server.registerTool(
  "nd_screenshot",
  {
    description: "Render the window in-process to a PNG at the given absolute path.",
    inputSchema: { path: z.string() },
  },
  async ({ path }) => ({ content: [{ type: "text" as const, text: JSON.stringify(await client.call("screenshot", { path })) }] }),
);

server.registerTool(
  "nd_click",
  {
    description: "Semantic click on a widget by ref (actionability-checked).",
    inputSchema: { ref: z.number() },
  },
  async ({ ref }) => ({ content: [{ type: "text" as const, text: JSON.stringify(await client.call("click", { ref })) }] }),
);

server.registerTool(
  "nd_wait_for",
  {
    description: "Wait until a tree condition holds (textContains or refVisible) or timeout.",
    inputSchema: {
      textContains: z.string().optional(),
      refVisible: z.number().optional(),
      timeoutMs: z.number().default(2000),
    },
  },
  async ({ textContains, refVisible, timeoutMs }) => {
    const condition = textContains !== undefined ? { textContains } : { refVisible };
    return {
      content: [{ type: "text" as const, text: JSON.stringify(await client.call("waitFor", { condition, timeoutMs })) }],
    };
  },
);

await server.connect(new StdioServerTransport());
