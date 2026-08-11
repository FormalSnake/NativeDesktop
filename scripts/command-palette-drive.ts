#!/usr/bin/env bun
// scripts/command-palette-drive.ts — drives examples/command-palette over the
// automation socket. The palette is mounted beside other content and the app
// re-renders on a background tick, so this proves the routed palette actions
// (type/click/setValue string|index|bool -> queryChanged/activate/submit) reach
// React reliably under the same churn that broke Return/click before the fix.
import { AutomationClient } from "@nativedesktop/test";

interface TreeNode {
  ref: number;
  type: string;
  testID: string | null;
  text: string | null;
  visible: boolean;
  children: TreeNode[];
}

function find(node: TreeNode, testID: string): TreeNode | null {
  if (node.testID === testID) return node;
  for (const child of node.children) {
    const found = find(child, testID);
    if (found) return found;
  }
  return null;
}

const client = await AutomationClient.connect();

const tree = (await client.call("getTree")) as { root: TreeNode };
function mustFind(testID: string): TreeNode {
  const n = find(tree.root, testID);
  if (!n) throw new Error(`${testID} not found in tree`);
  return n;
}

async function waitText(s: string): Promise<void> {
  const w = (await client.call("waitFor", { condition: { textContains: s }, timeoutMs: 4000 })) as { matched: boolean };
  if (!w.matched) throw new Error(`waitFor ${JSON.stringify(s)} did not match`);
}

const paletteRef = mustFind("palette").ref;
const openRef = mustFind("open-button").ref;

// 1. Open the palette (real button click), then confirm it presents — the
//    palette node reports visible only while presented.
await client.call("click", { ref: openRef });
const vis = (await client.call("waitFor", { condition: { refVisible: paletteRef }, timeoutMs: 4000 })) as { matched: boolean };
if (!vis.matched) throw new Error("palette did not present");

// 2. type -> queryChanged (append into the real search entry).
await client.call("type", { ref: paletteRef, text: "Dev" });
await waitText("Query: Dev");

// 3. click -> activate the highlighted row (the single "Developer" match): a
//    directory, so the app drills in and keeps the palette open.
await client.call("click", { ref: paletteRef });
await waitText("Folder: /Users/kyan/Developer");

// 4. setValue string -> replace the query text.
await client.call("setValue", { ref: paletteRef, value: "NativeDesktop" });
await waitText("Query: NativeDesktop");

// 5. setValue integer -> activate the row at that index (drills again).
await client.call("setValue", { ref: paletteRef, value: 0 });
await waitText("Folder: /Users/kyan/Developer/NativeDesktop");

// 6. setValue string then bool -> submit the raw query as-is (no matching row).
await client.call("setValue", { ref: paletteRef, value: "/tmp/typed-path" });
await waitText("Query: /tmp/typed-path");
await client.call("setValue", { ref: paletteRef, value: true });
await waitText("Picked: /tmp/typed-path");

console.log("CP_DRIVE_OK palette driven under churn: type/click/setValue string|index|submit");
client.close();
