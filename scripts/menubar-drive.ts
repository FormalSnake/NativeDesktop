#!/usr/bin/env bun
// scripts/menubar-drive.ts — drives examples/notes/menubar-probe.tsx over the
// automation socket (mirrors scripts/threepane-drive.ts / notes-drive.ts). M13
// Feature A: proves the <menubar>/<menu>/<menuitem> machinery on BOTH backends.
//
// (a) getTree surfaces the menubar/menu/menuitem nodes with their testIDs and
//     labels-as-text.
// (b) semanticClick on the File>New Thing menuitem fires onSelect → the counter
//     label increments (count=0 -> count=1).
// (c) the disabled item's semanticClick does NOT increment: the item's GAction
//     (GTK) / NSMenuItem validation (mac) blocks its onSelect, so a second
//     enabled click yields count=2, never count=102. Contract: a disabled
//     menuitem's click dispatches but its onSelect never fires.
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

const client = await AutomationClient.connect();

function tree(): Promise<GetTreeResult> {
  return client.call("getTree") as Promise<GetTreeResult>;
}

function mustFind(t: GetTreeResult, testID: string): TreeNode {
  const n = find(t.root, testID);
  if (!n) throw new Error(`${testID} not found in tree`);
  return n;
}

async function waitText(needle: string, ms = 3000): Promise<void> {
  const r = (await client.call("waitFor", { condition: { textContains: needle }, timeoutMs: ms })) as {
    matched: boolean;
  };
  if (!r.matched) throw new Error(`waitFor textContains ${JSON.stringify(needle)} did not match`);
}

// The initial counter label text is a stable settle signal.
await waitText("count=0");

// (a) menu tree present, labels surfaced as text.
const t0 = await tree();
mustFind(t0, "probe-menubar");
const fileMenu = mustFind(t0, "probe-file-menu");
const probeMenu = mustFind(t0, "probe-menu");
const newThing = mustFind(t0, "probe-new-thing");
const disabled = mustFind(t0, "probe-disabled");
mustFind(t0, "probe-about");
if (fileMenu.text !== "File") throw new Error(`file menu text=${JSON.stringify(fileMenu.text)} — want "File"`);
if (probeMenu.text !== "Probe") throw new Error(`probe menu text=${JSON.stringify(probeMenu.text)} — want "Probe"`);
if (newThing.text !== "New Thing") throw new Error(`new-thing text=${JSON.stringify(newThing.text)} — want "New Thing"`);
if (disabled.text !== "Disabled Item")
  throw new Error(`disabled text=${JSON.stringify(disabled.text)} — want "Disabled Item"`);

// (b) click New Thing → onSelect increments the counter.
await client.call("click", { ref: newThing.ref });
await waitText("count=1");

// (c) click the disabled item → onSelect must NOT fire (would add 100). An
// immediate getTree must still read count=1...
await client.call("click", { ref: disabled.ref });
const tAfterDisabled = await tree();
const counterAfterDisabled = mustFind(tAfterDisabled, "probe-counter");
if (counterAfterDisabled.text !== "count=1")
  throw new Error(`disabled click changed counter to ${JSON.stringify(counterAfterDisabled.text)} — want "count=1"`);

// ...and a second enabled click yields count=2 (never 102), settling the race:
// had the disabled onSelect fired (+100), this would be count=102 and the wait
// would time out.
await client.call("click", { ref: newThing.ref });
await waitText("count=2");

console.log(
  `ND_MENUBAR_PROBE_OK menubar+menu+menuitem in tree, onSelect fired (count=2), disabled click was a no-op`,
);
client.close();
