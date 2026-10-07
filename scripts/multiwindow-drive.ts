#!/usr/bin/env bun
// scripts/multiwindow-drive.ts: Portal + moveNode end to end on the
// Solid renderer. examples/multiwindow's <webview> loads a local page
// once, then moves from Window A to Window B and back. After each move the
// host tree has the webview under the slot it was moved to, and the page
// still carries the mark it was given on its first load, with no second load:
// the live widget moved and the page never reloaded.
// ND_HOST_BINARY picks the host (e.g. swift/.build/release/NDShell).
import type { JsonNode } from "../packages/react/src/core/generated/rpc.ts";
import { launchApp, expect, poll } from "../packages/test/src/index.ts";

const server = Bun.serve({
  port: 0,
  fetch: () => new Response("<!doctype html><title>tab</title><p>tab</p>", { headers: { "content-type": "text/html" } }),
});

function find(n: JsonNode, testID: string): JsonNode | undefined {
  if (n.testID === testID) return n;
  for (const c of n.children) {
    const hit = find(c, testID);
    if (hit) return hit;
  }
  return undefined;
}

let app: Awaited<ReturnType<typeof launchApp>> | undefined;
try {
  app = await launchApp({
    entry: "examples/multiwindow/main.tsx",
    hostBinary: process.env.ND_HOST_BINARY,
    env: { ND_DEMO_URL: `http://127.0.0.1:${server.port}/` },
  });
  const a = app;
  await a.waitForWindows(2);
  const state = a.getByTestId("tab-state");
  await expect(state).toHaveText("loads: 1 mark: set", { timeout: 15_000 });

  // The webview sits under `slot` in some window's tree.
  async function under(slot: string): Promise<void> {
    await poll(
      async () => {
        for (const w of (await a.windows()).windows) {
          const s = find((await a.tree(w.ref)).root, slot);
          if (s && find(s, "tab")) return true;
        }
        return false;
      },
      (ok) => ok,
      { timeoutMs: 5000 },
    );
  }

  await under("slot-a");
  await a.getByTestId("bring-b").click();
  await under("slot-b");
  await a.getByTestId("check").click();
  await expect(state).toHaveText("loads: 1 mark: kept");

  await a.getByTestId("bring-a").click();
  await under("slot-a");
  await a.getByTestId("check").click();
  await expect(state).toHaveText("loads: 1 mark: kept");

  console.log("ND_MULTIWINDOW_OK webview moved A -> B -> A with one page load and its page state intact");
} finally {
  await app?.close();
  server.stop(true);
}
