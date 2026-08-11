#!/usr/bin/env bun
// scripts/panes-drive.ts: drives examples/panes over the automation socket
// (mirrors scripts/errors-drive.ts's AutomationClient/find scaffold).
//
// Default mode builds the layout: split H, split V, close the third pane,
// assert the getTree shape (nested paned nodes, leaves in place), set the
// root ratio through the app button and assert the model echoes it, then
// leave time for the store debounce -> ND_PANES_OK.
// --restore (second host run against the same ND_STORE_DIR) asserts the
// persisted model came back: two panes, ratio 0.30, pane 3 gone ->
// ND_PANES_RESTORE_OK.
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

function mustFind(t: GetTreeResult, testID: string): TreeNode {
  const n = find(t.root, testID);
  if (!n) throw new Error(`${testID} not found in tree`);
  return n;
}

const client = await AutomationClient.connect();

async function tree(): Promise<GetTreeResult> {
  return (await client.call("getTree")) as GetTreeResult;
}

async function click(testID: string): Promise<void> {
  const t = await tree();
  const btn = mustFind(t, testID);
  const res = (await client.call("click", { ref: btn.ref })) as { dispatched: boolean };
  if (!res.dispatched) throw new Error(`${testID} click did not dispatch`);
}

async function waitForText(text: string, timeoutMs = 3000): Promise<void> {
  const waited = (await client.call("waitFor", { condition: { textContains: text }, timeoutMs })) as {
    matched: boolean;
  };
  if (!waited.matched) throw new Error(`waitFor ${JSON.stringify(text)} did not match`);
}

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

const restore = process.argv.includes("--restore");

if (!restore) {
  await waitForText("pane:1");

  await click("split-h-1"); // s2[1, 2]
  await waitForText("pane:2");
  await click("split-v-2"); // s2[1, s3[2, 3]]
  await waitForText("pane:3");

  const t = await tree();
  const outer = mustFind(t, "panes-split-s2");
  const inner = mustFind(t, "panes-split-s3");
  if (!find(outer, "panes-split-s3")) throw new Error("s3 is not nested inside s2");
  if (!find(inner, "panes-leaf-2") || !find(inner, "panes-leaf-3")) {
    throw new Error("inner split does not hold leaves 2 and 3");
  }
  mustFind(t, "panes-leaf-1");
  // Nested-layout regression gate (AppKit once collapsed BOTH outer panes to
  // width 0 when the inner split attached): every leaf must settle at real
  // geometry. Polled — the divider re-apply after a structural change is
  // deferred a runloop turn on AppKit, so the first tree read can race it.
  const leafIDs = ["panes-leaf-1", "panes-leaf-2", "panes-leaf-3"];
  let settled = false;
  for (let attempt = 0; attempt < 20 && !settled; attempt++) {
    const snap = await tree();
    settled = leafIDs.every((id) => {
      const g = find(snap.root, id)?.geometry;
      return g != null && g.w >= 50 && g.h >= 50;
    });
    if (!settled) await sleep(150);
  }
  if (!settled) {
    const snap = await tree();
    const report = leafIDs.map((id) => `${id}=${JSON.stringify(find(snap.root, id)?.geometry)}`).join(" ");
    throw new Error(`nested split leaves never got real geometry: ${report}`);
  }

  await click("close-3"); // back to s2[1, 2], focus moves to 2
  await waitForText("panes:2");
  const afterClose = await tree();
  if (find(afterClose.root, "pane-label-3")) throw new Error("pane 3 still present after close");
  if (find(afterClose.root, "panes-split-s3")) throw new Error("split s3 still present after close");
  if (!mustFind(afterClose, "pane-label-2").text?.includes("(focused)")) {
    throw new Error("focus did not move to the sibling after close");
  }

  await click("ratio-30");
  await waitForText("ratio:0.30");

  // Native positionChanged echo sanity: the left pane's box should sit near
  // 30% of the split's width (wide bounds; theming/handle width varies).
  const after = await tree();
  const leaf1 = mustFind(after, "panes-leaf-1");
  const split = mustFind(after, "panes-split-s2");
  if (leaf1.geometry && split.geometry && split.geometry.w > 0) {
    const frac = leaf1.geometry.w / split.geometry.w;
    if (frac < 0.15 || frac > 0.45) {
      throw new Error(`left pane fraction ${frac.toFixed(2)} not near 0.30 after ratio write`);
    }
  }

  await sleep(600); // let the debounced ratio write land before the host is killed

  console.log("ND_PANES_OK");
  client.close();
  process.exit(0);
}

// --restore
await waitForText("pane:1");
await waitForText("pane:2");
await waitForText("panes:2");
await waitForText("ratio:0.30");
const t = await tree();
mustFind(t, "panes-split-s2");
mustFind(t, "panes-leaf-1");
mustFind(t, "panes-leaf-2");
if (find(t.root, "pane-label-3")) throw new Error("pane 3 resurrected after restore");
console.log("ND_PANES_RESTORE_OK");
client.close();
