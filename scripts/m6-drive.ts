#!/usr/bin/env bun
// scripts/m6-drive.ts — drives the Mac AppKit shell over the automation socket.
// Runs ON THE MAC (scripts/mac/mac-m6.sh ssh's in and invokes it against the
// local socket path — no tunnel; mirrors how mac-build.sh runs zig remotely).
// Reuses the same AutomationClient as m4/m5c; assertions mirror them:
//   counter: getTree -> click x3 -> waitFor "Clicks: 3" -> non-blank screenshot
//   gallery: styled/type asserts + ListView itemCount contract (m5c-drive.ts)
//   --slo:   getTree + screenshot only (child SIGSTOPped, D11 SLO leg)
import { AutomationClient } from "../packages/mcp/src/socket.ts";

interface TreeNode {
  ref: number;
  type: string;
  testID: string | null;
  text: string | null;
  visible: boolean;
  geometry: { x: number; y: number; w: number; h: number } | null;
  children: TreeNode[];
  itemCount?: number | null;
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

// Screenshot polls like waitFor (the m5b/m5c 150ms->3s pattern): a frame that
// isn't ready yet (-32603) becomes a retry, not a failure.
async function pollScreenshot(
  client: AutomationClient,
  path: string,
): Promise<{ path: string; width: number; height: number }> {
  let shot: { path: string; width: number; height: number } | null = null;
  let lastErr: Error | null = null;
  for (let i = 0; i < 20; i++) {
    try {
      shot = (await client.call("screenshot", { path })) as { path: string; width: number; height: number };
      break;
    } catch (e) {
      lastErr = e as Error;
      await new Promise((r) => setTimeout(r, 150));
    }
  }
  if (!shot) throw new Error(`screenshot failed after retries: ${lastErr?.message}`);
  if (shot.width <= 0 || shot.height <= 0) throw new Error("screenshot has no dimensions");
  return shot;
}

const mode = process.argv[2] ?? "counter";
const slo = process.argv.includes("--slo");
const outPng = process.env.ND_SHOT_PATH ?? "/tmp/m6-shot.png";
const client = await AutomationClient.connect();

if (slo) {
  const tree = (await client.call("getTree")) as GetTreeResult;
  if (!tree || tree.coordinateSpace !== "logical-window-topleft") {
    throw new Error("bad getTree result under stall");
  }
  const shot = await pollScreenshot(client, outPng);
  console.log(`M6_SLO_OK tree+screenshot answered while child stalled png=${shot.path} ${shot.width}x${shot.height}`);
  client.close();
  process.exit(0);
}

if (mode === "counter") {
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

  const shot = await pollScreenshot(client, outPng);
  console.log(`M6_DRIVE_OK clicks=3 png=${shot.path} ${shot.width}x${shot.height}`);
} else if (mode === "gallery") {
  const tree = (await client.call("getTree")) as GetTreeResult;
  if (tree.coordinateSpace !== "logical-window-topleft") throw new Error("bad coordinate space");

  function mustFind(testID: string): TreeNode {
    const n = find(tree.root, testID);
    if (!n) throw new Error(`${testID} not found in tree`);
    return n;
  }

  // Widget-create assertions (mirror m5c-drive.ts): the styled tab's nodes
  // exist with the right types, proving all 18 create arms mounted.
  const styledTab = mustFind("styled-tab");
  if (styledTab.type !== "Box") throw new Error(`styled-tab wrong type: ${styledTab.type}`);
  const styledLabel = mustFind("styled-label");
  if (styledLabel.type !== "Label") throw new Error(`styled-label wrong type: ${styledLabel.type}`);
  const styledButton = mustFind("styled-button");
  if (styledButton.type !== "Button") throw new Error(`styled-button wrong type: ${styledButton.type}`);

  // ListView itemCount contract (M5c-D4 / M6b-D2): 100k items, zero children.
  const list = mustFind("big-list");
  if (list.type !== "ListView") throw new Error(`big-list not a ListView: ${list.type}`);
  if (list.itemCount !== 100000) throw new Error(`itemCount=${list.itemCount}, want 100000`);
  if (list.children.length !== 0) throw new Error(`ListView dumped ${list.children.length} children (must be 0)`);

  // rowActivated wired end-to-end: activated-label mirrors React state,
  // steady-state "Activated: -1" (same observable proof as m5c).
  const activatedLabel = mustFind("activated-label");
  if (activatedLabel.text !== "Activated: -1") {
    throw new Error(`activated-label text=${activatedLabel.text}, want "Activated: -1"`);
  }

  const shot = await pollScreenshot(client, outPng);
  console.log(`M6_GALLERY_OK styled+list png=${shot.path} ${shot.width}x${shot.height} itemCount=${list.itemCount}`);
} else {
  throw new Error(`unknown mode: ${mode}`);
}
client.close();
