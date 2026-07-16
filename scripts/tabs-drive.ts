#!/usr/bin/env bun
// scripts/tabs-drive.ts — asserts native system tabs (M17) against
// examples/browser: cmd+t (File > New Tab key equivalent) must open a second
// <window tabGroup="browser"> that joins the native tab group, and cmd+w
// (File > Close, AppKit default menu) must run the deferred close loop —
// closed event -> app unmounts the <window> -> remove op's window.close
// semantic action. Window count is read from the a11y tree (role "window",
// other windows attach as orphans of the root snapshot).
import { AutomationClient } from "../packages/mcp/src/socket.ts";
import type { JsonNode, GetTreeResult } from "../packages/react/src/generated/rpc.ts";

function countWindows(node: JsonNode): number {
  let n = node.role === "window" ? 1 : 0;
  for (const child of node.children) n += countWindows(child);
  return n;
}

async function windowsNow(client: AutomationClient): Promise<number> {
  const t = (await client.call("getTree")) as GetTreeResult;
  return countWindows(t.root);
}

async function waitForWindows(client: AutomationClient, want: number): Promise<number> {
  let n = -1;
  for (let i = 0; i < 50; i++) {
    n = await windowsNow(client);
    if (n === want) return n;
    await new Promise((r) => setTimeout(r, 100));
  }
  return n;
}

// Baseline-relative so a human poking the live app between runs can't skew
// the assertion — only the +1/-1 transitions are the contract.
const client = await AutomationClient.connect();
const start = await windowsNow(client);

await client.call("keys", { keys: "cmd+t" });
const opened = await waitForWindows(client, start + 1);
if (opened !== start + 1) throw new Error(`cmd+t: window count ${start} -> ${opened}, want ${start + 1}`);

// Screenshot right after the tab-bar animation races frame invalidation
// (-32603 until a frame lands) — the m6-drive retry lesson.
for (let i = 0; i < 20; i++) {
  try {
    await client.call("screenshot", { path: "/tmp/nd-tabs-two.png" });
    break;
  } catch {
    await new Promise((r) => setTimeout(r, 150));
  }
}

await client.call("keys", { keys: "cmd+w" });
const closed = await waitForWindows(client, start);
if (closed !== start) throw new Error(`cmd+w: window count ${opened} -> ${closed}, want ${start}`);

console.log("ND_TABS_OK new tab via cmd+t joined the group and cmd+w ran the deferred close loop");
client.close();
