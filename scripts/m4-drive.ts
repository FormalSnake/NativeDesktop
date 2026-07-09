#!/usr/bin/env bun
// scripts/m4-drive.ts — connects ND_AUTOMATION_SOCKET, drives the counter, asserts, screenshots.
//
// Two modes:
//   default: getTree -> find "increment-button" -> click x3 -> waitFor "Clicks: 3" -> screenshot -> assert
//   --slo:   getTree + screenshot only (used while the JS child is SIGSTOPped for the D11 SLO test)
import { AutomationClient } from "../packages/mcp/src/socket.ts";

interface TreeNode {
  ref: number;
  type: string;
  testID: string | null;
  text: string | null;
  visible: boolean;
  geometry: { x: number; y: number; w: number; h: number } | null;
  children: TreeNode[];
}

interface GetTreeResult {
  coordinateSpace: string;
  root: TreeNode;
}

function find(node: TreeNode, testID: string): TreeNode | null {
  if (node.testID === testID) return node;
  for (const child of node.children) {
    const found = find(child, testID);
    if (found) return found;
  }
  return null;
}

const slo = process.argv.includes("--slo");
const outPng = process.env.ND_SHOT_PATH ?? "/tmp/m4-shot.png";
const client = await AutomationClient.connect();

if (slo) {
  const tree = (await client.call("getTree")) as GetTreeResult;
  if (!tree || tree.coordinateSpace !== "logical-window-topleft") {
    throw new Error("bad getTree result under stall");
  }
  const shot = (await client.call("screenshot", { path: outPng })) as {
    path: string;
    width: number;
    height: number;
  };
  if (shot.width <= 0 || shot.height <= 0) throw new Error("screenshot has no dimensions under stall");
  console.log(`M4_SLO_OK tree+screenshot answered while child stalled png=${shot.path} ${shot.width}x${shot.height}`);
  client.close();
  process.exit(0);
}

const tree = (await client.call("getTree")) as GetTreeResult;
if (tree.coordinateSpace !== "logical-window-topleft") throw new Error("bad coordinate space");
const btn = find(tree.root, "increment-button");
if (!btn) throw new Error("increment-button not found in tree");

for (let i = 0; i < 3; i++) {
  const res = (await client.call("click", { ref: btn.ref })) as { ref: number; dispatched: boolean };
  if (!res.dispatched) throw new Error(`click ${i + 1} did not dispatch`);
}

const waited = (await client.call("waitFor", {
  condition: { textContains: "Clicks: 3" },
  timeoutMs: 3000,
})) as { matched: boolean };
if (!waited.matched) throw new Error("waitFor Clicks: 3 did not match");

const shot = (await client.call("screenshot", { path: outPng })) as {
  path: string;
  width: number;
  height: number;
};
if (shot.width <= 0 || shot.height <= 0) throw new Error("screenshot has no dimensions");

console.log(`M4_DRIVE_OK clicks=3 png=${shot.path} ${shot.width}x${shot.height}`);
client.close();
