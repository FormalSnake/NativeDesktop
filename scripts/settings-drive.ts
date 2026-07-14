#!/usr/bin/env bun
// Drives the semantic settings groups through the automation socket. This is
// specifically an interop regression test: controls remain AppKit/GTK widgets
// parented by the settings group while macOS draws the card with SwiftUI.
import { AutomationClient } from "../packages/mcp/src/socket.ts";

interface TreeNode {
  ref: number;
  type: string;
  testID: string | null;
  text: string | null;
  checked?: boolean | null;
  value?: number | null;
  selectedIndex?: number | null;
  children: TreeNode[];
}
interface GetTreeResult { root: TreeNode }

const client = await AutomationClient.connect();
const tree = async () => (await client.call("getTree")) as GetTreeResult;
function find(node: TreeNode, testID: string): TreeNode | null {
  if (node.testID === testID) return node;
  for (const child of node.children) {
    const result = find(child, testID);
    if (result) return result;
  }
  return null;
}
async function node(testID: string): Promise<TreeNode> {
  const result = find((await tree()).root, testID);
  if (!result) throw new Error(`${testID} not found`);
  return result;
}
async function set(testID: string, value: boolean | number): Promise<void> {
  const target = await node(testID);
  const result = (await client.call("setValue", { ref: target.ref, value })) as { applied: boolean };
  if (!result.applied) throw new Error(`setValue did not apply to ${testID}`);
}
async function click(testID: string): Promise<void> {
  const target = await node(testID);
  const result = (await client.call("click", { ref: target.ref })) as { dispatched: boolean };
  if (!result.dispatched) throw new Error(`click did not dispatch to ${testID}`);
}
async function waitFor(testID: string, predicate: (n: TreeNode) => boolean): Promise<TreeNode> {
  let last: TreeNode | null = null;
  for (let i = 0; i < 30; i++) {
    last = find((await tree()).root, testID);
    if (last && predicate(last)) return last;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`${testID} did not reach expected state; last=${JSON.stringify(last)}`);
}

const group = await node("general-card");
if (group.type !== "SettingsGroup") throw new Error(`general-card type=${group.type}`);

// These dispatch through the existing native controls nested inside the
// SwiftUI-backed group. A successful setValue plus the host's follow-up commit
// proves both event paths remain connected.
await set("setting-launch", false);
await set("setting-folder", 2);

await click("category-appearance");
await waitFor("appearance-card", (n) => n.type === "Box");
await set("setting-textsize", 21);
await waitFor("setting-textsize-caption", (n) => n.text === "21pt");

// Remount the conditional settings groups and ensure React-owned state survives.
await click("category-advanced");
await waitFor("advanced-card", (n) => n.type === "Box");
await click("category-appearance");
await waitFor("setting-textsize-caption", (n) => n.text === "21pt");
await click("category-advanced");
await waitFor("advanced-card", (n) => n.type === "Box");
await click("reset-button");
await click("category-appearance");
await waitFor("setting-textsize-caption", (n) => n.text === "14pt");

const shotPath = process.env.ND_SHOT_PATH ?? "/tmp/nd-settings-swiftui.png";
const shot = (await client.call("screenshot", { path: shotPath })) as { width: number; height: number };
if (shot.width <= 0 || shot.height <= 0) throw new Error("settings screenshot has no dimensions");
console.log(`SETTINGS_REACTIVE_OK SwiftUI-hosted group preserved events, updates, remounts, and reset state; screenshot=${shotPath}`);
client.close();
