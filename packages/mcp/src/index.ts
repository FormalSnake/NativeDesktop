#!/usr/bin/env bun
// Stdio MCP server wrapping the NativeDesktop automation socket (ND_AUTOMATION_SOCKET).
// Exposes the full automation RPC surface as agent tools, each a thin pass-through to
// the framed JSON-RPC automation server (see docs/superpowers/plans
// /2026-07-09-m4-automation.md for v1; input synthesis + a11y tree are M16).
// pointer/drag/keys/double/right-click/hover post real native events on macOS and
// answer -32003 on GTK (no in-process event synthesis on GTK4).

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import { AutomationClient } from "./socket.ts";

const client = await AutomationClient.connect();
const server = new McpServer({ name: "nativedesktop", version: "0.0.0" });

server.registerTool(
  "nd_get_tree",
  {
    description:
      "Snapshot the accessibility tree: stable refs, testIDs, text, logical geometry, role, enabled, focused, and live value per node. Optional window ref scopes the snapshot to one window.",
    inputSchema: { window: z.number().optional() },
  },
  async ({ window }) => ({
    content: [
      {
        type: "text" as const,
        text: JSON.stringify(await client.call("getTree", window !== undefined ? { window } : undefined), null, 2),
      },
    ],
  }),
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

server.registerTool(
  "nd_set_value",
  {
    description:
      "Set a widget's value semantically (fires the native change event): string for TextInput/TextArea, boolean for Checkbox/Radio/Switch, number for Slider, integer index for Select/SourceList.",
    inputSchema: { ref: z.number(), value: z.union([z.string(), z.number(), z.boolean()]) },
  },
  async ({ ref, value }) => ({
    content: [{ type: "text" as const, text: JSON.stringify(await client.call("setValue", { ref, value })) }],
  }),
);

server.registerTool(
  "nd_type",
  {
    description: "Append text to a TextInput semantically (never synthetic keysyms); returns the full text after the insert.",
    inputSchema: { ref: z.number(), text: z.string() },
  },
  async ({ ref, text }) => ({
    content: [{ type: "text" as const, text: JSON.stringify(await client.call("type", { ref, text })) }],
  }),
);

server.registerTool(
  "nd_scroll",
  {
    description: "Scroll a ScrollView by dx/dy logical units; returns the resulting offsets.",
    inputSchema: { ref: z.number(), dx: z.number().optional(), dy: z.number().optional() },
  },
  async ({ ref, dx, dy }) => ({
    content: [{ type: "text" as const, text: JSON.stringify(await client.call("scroll", { ref, dx, dy })) }],
  }),
);

server.registerTool(
  "nd_double_click",
  {
    description: "Double-click a widget's center via real input synthesis (activates table/list rows). macOS only (-32003 on GTK).",
    inputSchema: { ref: z.number() },
  },
  async ({ ref }) => ({
    content: [{ type: "text" as const, text: JSON.stringify(await client.call("doubleClick", { ref })) }],
  }),
);

server.registerTool(
  "nd_right_click",
  {
    description:
      "Right-click a widget's center via real input synthesis; an opened context menu is auto-dismissed to keep automation responsive. macOS only (-32003 on GTK).",
    inputSchema: { ref: z.number() },
  },
  async ({ ref }) => ({
    content: [{ type: "text" as const, text: JSON.stringify(await client.call("rightClick", { ref })) }],
  }),
);

server.registerTool(
  "nd_hover",
  {
    description: "Move the synthetic pointer over a widget's center (best-effort hover). macOS only (-32003 on GTK).",
    inputSchema: { ref: z.number() },
  },
  async ({ ref }) => ({
    content: [{ type: "text" as const, text: JSON.stringify(await client.call("hover", { ref })) }],
  }),
);

server.registerTool(
  "nd_pointer",
  {
    description:
      "Low-level pointer phase (down|move|up) at logical-window-topleft coordinates; clickCount 2 on down+up makes a double-click. Prefer nd_drag for press-move-release. macOS only (-32003 on GTK).",
    inputSchema: {
      phase: z.enum(["down", "move", "up"]),
      x: z.number(),
      y: z.number(),
      button: z.enum(["left", "right"]).optional(),
      clickCount: z.number().optional(),
      window: z.number().optional(),
    },
  },
  async (params) => ({
    content: [{ type: "text" as const, text: JSON.stringify(await client.call("pointer", params)) }],
  }),
);

server.registerTool(
  "nd_drag",
  {
    description:
      "Press-move-release drag between two widget refs (their centers) or explicit coordinates — drives slider thumbs, split dividers, selections like a real mouse. macOS only (-32003 on GTK).",
    inputSchema: {
      fromRef: z.number().optional(),
      toRef: z.number().optional(),
      fromX: z.number().optional(),
      fromY: z.number().optional(),
      toX: z.number().optional(),
      toY: z.number().optional(),
      steps: z.number().optional(),
      durationMs: z.number().optional(),
      window: z.number().optional(),
    },
  },
  async (params) => ({
    content: [{ type: "text" as const, text: JSON.stringify(await client.call("drag", params)) }],
  }),
);

server.registerTool(
  "nd_keys",
  {
    description:
      "Keyboard synthesis: a chord like 'cmd+n' presses one combination (drives menu key equivalents); a plain string like 'hello' types each character into the focused widget. macOS only (-32003 on GTK).",
    inputSchema: { keys: z.string(), window: z.number().optional() },
  },
  async (params) => ({
    content: [{ type: "text" as const, text: JSON.stringify(await client.call("keys", params)) }],
  }),
);

await server.connect(new StdioServerTransport());
