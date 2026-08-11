#!/usr/bin/env bun
// scripts/m5b-drive.ts — drives the gallery over the automation socket:
// setValue/type/scroll semantic actions -> React state round-trips -> waitFor.
import { AutomationClient } from "@nativedesktop/test";

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

const outPng = process.env.ND_SHOT_PATH ?? "/tmp/m5b-shot.png";
const client = await AutomationClient.connect();

const tree = (await client.call("getTree")) as GetTreeResult;
if (tree.coordinateSpace !== "logical-window-topleft") throw new Error("bad coordinate space");

function mustFind(testID: string): TreeNode {
  const n = find(tree.root, testID);
  if (!n) throw new Error(`${testID} not found in tree`);
  return n;
}

async function waitText(s: string): Promise<void> {
  const w = (await client.call("waitFor", { condition: { textContains: s }, timeoutMs: 3000 })) as { matched: boolean };
  if (!w.matched) throw new Error(`waitFor ${JSON.stringify(s)} did not match`);
}

// 1. TextInput: setValue -> changed event -> React state -> bound label.
await client.call("setValue", { ref: mustFind("name-input").ref, value: "hello" });
await waitText("Echo: hello");

// 2. type appends through GtkEditable (semantic, not keysyms).
const typed = (await client.call("type", { ref: mustFind("name-input").ref, text: " world" })) as { text: string };
if (typed.text !== "hello world") throw new Error(`type result text=${typed.text}`);
await waitText("Echo: hello world");

// 3. TextArea via TextBuffer.
await client.call("setValue", { ref: mustFind("notes-area").ref, value: "some notes" });
await waitText("Notes: some notes");

// 4. Checkbox toggled.
await client.call("setValue", { ref: mustFind("agree-check").ref, value: true });
await waitText("Agreed: yes");

// 5. Radio group: activating "large" must flow through toggled -> state.
await client.call("setValue", { ref: mustFind("size-large").ref, value: true });
await waitText("Size: large");

// 6. Slider (GtkAdjustment) -> valueChanged -> label + progressbar fraction.
await client.call("setValue", { ref: mustFind("volume-slider").ref, value: 75 });
await waitText("Volume: 75");

// 7. Select (notify::selected) -> selectionChanged.
await client.call("setValue", { ref: mustFind("fruit-select").ref, value: 2 });
await waitText("Fruit: cherry");

// 8. Scroll the ScrollView via its vertical adjustment.
const scrolled = (await client.call("scroll", { ref: mustFind("log-scroll").ref, dy: 200 })) as { x: number; y: number };
if (!(scrolled.y > 0)) throw new Error(`scroll did not move vadjustment (y=${scrolled.y})`);

// 9. Structure: grid children present even in a background tab; WebView stub in tree.
const grid = mustFind("layout-grid");
if (grid.children.length !== 3) throw new Error(`grid has ${grid.children.length} children, want 3`);
if (mustFind("web-stub").type !== "WebView") throw new Error("web-stub node is not a WebView");

// 10. Error contract: setValue on a Label must be -32602, not a crash.
let rejected = false;
try {
  await client.call("setValue", { ref: mustFind("echo-label").ref, value: "nope" });
} catch {
  rejected = true; // AutomationClient surfaces JSON-RPC errors as throws (m4 pattern)
}
if (!rejected) throw new Error("setValue on a Label should be rejected");

// Screenshot polls like waitFor: right after the scroll the window's rendered
// frame is invalidated, and GtkWidgetPaintable reports "empty snapshot"
// (-32603) until the next frame lands (verified: attempt 0 fails, +200ms OK).
let shot: { path: string; width: number; height: number } | null = null;
let lastErr: Error | null = null;
for (let i = 0; i < 20; i++) {
  try {
    shot = (await client.call("screenshot", { path: outPng })) as { path: string; width: number; height: number };
    break;
  } catch (e) {
    lastErr = e as Error;
    await new Promise((r) => setTimeout(r, 150));
  }
}
if (!shot) throw new Error(`screenshot failed after retries: ${lastErr?.message}`);
if (shot.width <= 0 || shot.height <= 0) throw new Error("screenshot has no dimensions");

console.log(`M5B_DRIVE_OK gallery driven png=${shot.path} ${shot.width}x${shot.height}`);
client.close();
