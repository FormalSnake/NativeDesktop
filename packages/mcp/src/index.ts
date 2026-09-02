#!/usr/bin/env bun
// Stdio MCP server for driving a NativeDesktop app.
//
// Three tools, not one per RPC. An agent writes @nativedesktop/test code
// against a live app and gets the value back, the same shape a drive script
// would take, so anything the harness can express is reachable without a new
// tool. `snapshot` is the cheap way to see what is on screen, and `reset`
// puts a wedged or crashed host back.
//
// Two modes. With ND_AUTOMATION_SOCKET set the server attaches to a host
// somebody else launched; otherwise it spawns one from ND_MCP_ENTRY.
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import {
  asNdNode,
  AttachedApp,
  AutomationRpcError,
  expect,
  launchApp,
  Locator,
  renderSnapshot,
  type AppHandle,
} from "@nativedesktop/test";
import type { Backend } from "@nativedesktop/host";

const DEFAULT_EXECUTE_TIMEOUT_MS = 30_000;
const SNAPSHOT_MAX_LINES = 400;

type App = AppHandle | AttachedApp;

interface SessionOptions {
  entry: string;
  backend?: Backend;
}

let options: SessionOptions = {
  entry: process.env.ND_MCP_ENTRY ?? "",
  backend: process.env.ND_MCP_BACKEND as Backend | undefined,
};
let app: App | undefined;
/** Survives across execute calls so an agent can stash a ref, a window, or a
 * half-built fixture between one-liners. */
let state: Record<string, unknown> = {};

const attached = Boolean(process.env.ND_AUTOMATION_SOCKET);

async function open(): Promise<App> {
  if (attached) return AttachedApp.connect();
  if (!options.entry) {
    throw new Error("no app to drive: set ND_MCP_ENTRY (or ND_AUTOMATION_SOCKET to attach to a running host)");
  }
  return launchApp({ entry: options.entry, backend: options.backend });
}

async function currentApp(): Promise<App> {
  if (!app) app = await open();
  return app;
}

/** An attached host's stderr belongs to whoever launched it. */
function stderrTail(target: App): string {
  return target instanceof AttachedApp ? "(attached host: its stderr is not ours to read)" : target.stderrTail(40);
}

/** Result values an agent can act on: a Locator is worth its selector, a tree
 * node its identity, and a cycle is not worth a stack overflow. */
function serialize(value: unknown, seen = new WeakSet<object>()): unknown {
  if (value instanceof Locator) return value.selector;
  if (value instanceof AutomationRpcError) return { error: value.message, code: value.code, data: value.data };
  if (value instanceof Error) return { error: value.message };
  if (value === null || typeof value !== "object") return value;
  if (seen.has(value)) return "[Circular]";
  seen.add(value);
  if (Array.isArray(value)) return value.map((v) => serialize(v, seen));
  const node = value as Record<string, unknown>;
  if (typeof node.ref === "number" && typeof node.type === "string" && Array.isArray(node.children)) {
    return { ref: node.ref, type: node.type, role: node.role, text: node.text, testID: node.testID };
  }
  const out: Record<string, unknown> = {};
  for (const [key, inner] of Object.entries(node)) out[key] = serialize(inner, seen);
  return out;
}

const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor as new (
  ...args: string[]
) => (...args: unknown[]) => Promise<unknown>;

/** Index of the last `;` outside a string, template, bracket or brace. */
function lastTopLevelSemicolon(code: string): number {
  let depth = 0;
  let quote: string | null = null;
  let found = -1;
  for (let i = 0; i < code.length; i++) {
    const c = code[i]!;
    if (quote) {
      if (c === "\\") i++;
      else if (c === quote) quote = null;
      continue;
    }
    if (c === '"' || c === "'" || c === "`") quote = c;
    else if (c === "(" || c === "[" || c === "{") depth++;
    else if (c === ")" || c === "]" || c === "}") depth--;
    else if (c === ";" && depth === 0) found = i;
  }
  return found;
}

function compile(code: string, names: string[]): (...args: unknown[]) => Promise<unknown> {
  try {
    // Expression mode first, so a bare `await app.getByTestId("x").count()`
    // answers with the count instead of undefined.
    return new AsyncFunction(...names, `return (${code}\n);`);
  } catch {
    // Statement mode. The last statement is still returned where it is an
    // expression, so a chain of setup calls ending in a read answers with
    // the read rather than nothing.
    const body = code.trim().replace(/;\s*$/, "");
    const cut = lastTopLevelSemicolon(body);
    if (cut > 0) {
      try {
        return new AsyncFunction(...names, `${body.slice(0, cut)};\nreturn (${body.slice(cut + 1)}\n);`);
      } catch {
        // The tail is a declaration or a control-flow statement, not a value.
      }
    }
    return new AsyncFunction(...names, code);
  }
}

/** Console output belongs in the tool result, not on stdout: stdout is the
 * MCP transport, and one stray log frames the protocol out of sync. */
function captureConsole(logs: string[]): () => void {
  const methods = ["log", "info", "warn", "error", "debug"] as const;
  const saved = methods.map((m) => [m, console[m]] as const);
  for (const method of methods) {
    console[method] = (...args: unknown[]): void => {
      logs.push(args.map((a) => (typeof a === "string" ? a : Bun.inspect(a))).join(" "));
    };
  }
  return () => {
    for (const [method, fn] of saved) console[method] = fn;
  };
}

function text(value: unknown): { content: { type: "text"; text: string }[] } {
  return {
    content: [{ type: "text" as const, text: typeof value === "string" ? value : JSON.stringify(value, null, 2) }],
  };
}

const server = new McpServer({ name: "nativedesktop", version: "0.1.0" });

server.registerTool(
  "execute",
  {
    description:
      "Control the user's NativeDesktop app via @nativedesktop/test code snippets. Prefer single-line code with semicolons between statements.",
    inputSchema: {
      code: z
        .string()
        .describe(
          "js @nativedesktop/test code, has {app, state, expect, launchApp, snapshot} in scope. Should be one line, using ; to execute multiple statements. you MUST call execute multiple times instead of writing complex scripts in a single tool call.",
        ),
      timeout: z.number().optional().describe("ms before the snippet is abandoned (default 30000)"),
    },
  },
  async ({ code, timeout }) => {
    let target: App;
    try {
      target = await currentApp();
    } catch (e) {
      return text({ error: (e as Error).message, hint: "call reset" });
    }
    if (!target.isAlive()) {
      return text({ error: "the host process is gone", stderrTail: stderrTail(target), hint: "call reset" });
    }
    const logs: string[] = [];
    const restore = captureConsole(logs);
    const names = ["app", "state", "expect", "launchApp", "snapshot"];
    try {
      const fn = compile(code, names);
      const run = fn(target, state, expect, launchApp, (opts?: { window?: number; interactiveOnly?: boolean }) =>
        renderTree(target, opts ?? {}),
      );
      const result = await Promise.race([
        run,
        new Promise<never>((_, reject) =>
          setTimeout(() => reject(new Error(`execute timed out after ${timeout ?? DEFAULT_EXECUTE_TIMEOUT_MS}ms`)),
            timeout ?? DEFAULT_EXECUTE_TIMEOUT_MS),
        ),
      ]);
      return text({ result: serialize(result) ?? null, logs });
    } catch (e) {
      const error = e as Error;
      if (error instanceof AutomationRpcError) {
        return text({ error: error.message, code: error.code, data: error.data, logs });
      }
      const dead = !target.isAlive();
      return text({
        error: error.message,
        logs,
        ...(dead ? { stderrTail: stderrTail(target), hint: "call reset" } : {}),
      });
    } finally {
      restore();
    }
  },
);

async function renderTree(target: App, opts: { window?: number; interactiveOnly?: boolean }): Promise<string> {
  const tree = await target.tree(opts.window);
  return renderSnapshot(asNdNode(tree.root), {
    interactiveOnly: opts.interactiveOnly ?? false,
    maxLines: SNAPSHOT_MAX_LINES,
  });
}

server.registerTool(
  "snapshot",
  {
    description:
      "Compact accessibility tree of the app: one line per node, indented by depth, with the wire ref every action targets. Read this before acting, the way you would read a page.",
    inputSchema: {
      window: z.number().optional().describe("Window node ref, from app.windows(); absent means the root window"),
      interactiveOnly: z.boolean().optional().describe("Keep only widgets an action can reach"),
    },
  },
  async ({ window, interactiveOnly }) => {
    try {
      const target = await currentApp();
      return text(await renderTree(target, { window, interactiveOnly }));
    } catch (e) {
      return text({ error: (e as Error).message, hint: "call reset" });
    }
  },
);

server.registerTool(
  "reset",
  {
    description:
      "Close the app and launch it again, clearing `state`. Use it after a crash, a wedged dialog, or to start a scenario from a known screen. Attaching to a running host (ND_AUTOMATION_SOCKET) reconnects instead of respawning.",
    inputSchema: {
      entry: z.string().optional().describe("entry file to launch, e.g. examples/counter/main.tsx"),
      backend: z.enum(["appkit", "gtk"]).optional(),
    },
  },
  async ({ entry, backend }) => {
    if (entry) options = { ...options, entry };
    if (backend) options = { ...options, backend };
    await app?.close().catch(() => {});
    app = undefined;
    state = {};
    try {
      const target = await currentApp();
      return text({
        mode: attached ? "attached" : "spawned",
        pid: target instanceof AttachedApp ? null : target.pid,
        socket: target instanceof AttachedApp ? process.env.ND_AUTOMATION_SOCKET : target.socketPath,
        entry: options.entry || null,
      });
    } catch (e) {
      return text({ error: (e as Error).message });
    }
  },
);

await server.connect(new StdioServerTransport());
