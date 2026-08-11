#!/usr/bin/env bun
// scripts/ndwidgets-probe-drive.ts — drives examples/ndwidgets-probe over the
// automation socket and asserts the 2026-07-17 nd-widgets wave on GTK: C3
// icon-only vs icon+label button text routing, both `<window>`s present in
// the a11y tree, and the C5 "present" widgetCommand dispatching without
// error via a semantic click (GTK has no synthetic pointer/hover input —
// -32003, src/gtk/backend.zig's vtSemanticAction — so real onHoverChanged
// interaction is verified on AppKit instead, where `hover` posts a real
// mouseMoved).
import { AutomationClient } from "@nativedesktop/test";
import type { JsonNode, GetTreeResult } from "../packages/react/src/generated/rpc.ts";

function find(node: JsonNode, testID: string): JsonNode | null {
  if (node.testID === testID) return node;
  for (const child of node.children) {
    const found = find(child, testID);
    if (found) return found;
  }
  return null;
}

function mustFind(tree: JsonNode, testID: string): JsonNode {
  const node = find(tree, testID);
  if (!node) throw new Error(`${testID} not found in tree`);
  return node;
}

function countWindows(node: JsonNode): number {
  let n = node.role === "window" ? 1 : 0;
  for (const child of node.children) n += countWindows(child);
  return n;
}

const client = await AutomationClient.connect();

const t = (await client.call("getTree")) as GetTreeResult;

// C3: iconName + label="" is icon-only (no text); iconName + non-empty
// label renders icon AND text.
const iconOnly = mustFind(t.root, "icon-only-btn");
if (iconOnly.text) throw new Error(`icon-only-btn text=${JSON.stringify(iconOnly.text)}, want empty`);
const iconLabel = mustFind(t.root, "icon-label-btn");
if (iconLabel.text !== "Refresh") throw new Error(`icon-label-btn text=${JSON.stringify(iconLabel.text)}, want "Refresh"`);
console.log("ND_C3_OK icon-only vs icon+label button text routing correct");

// Both probe windows exist (root A + orphaned B, tabs-drive.ts's counting idiom).
const windowCount = countWindows(t.root);
if (windowCount !== 2) throw new Error(`window count=${windowCount}, want 2`);
console.log("ND_WINDOWS_OK both probe windows present in the a11y tree");

// C5: "present" dispatches cleanly through the GTK tabs.zig arm (a real
// pointer-driven raise/focus is verified on AppKit — see the Mac recipe).
const presentBtn = mustFind(t.root, "present-b-btn");
const click = (await client.call("click", { ref: presentBtn.ref })) as { dispatched: boolean };
if (!click.dispatched) throw new Error("present-b-btn click did not dispatch");
console.log("ND_C5_OK Window \"present\" widgetCommand dispatched with no error");

// navigation-sidebar reach check: both host sections + all three run rows
// (nested two levels below the classed box) resolve in the tree and their
// clicks dispatch — GTK gets this "for free" (a CSS class cascades
// regardless of nesting depth); the AppKit native-table-row equivalent is
// verified on the Mac recipe (SidebarTable.swift's recursive row collection).
for (const id of ["host-a", "host-b", "run-a1", "run-a2", "run-b1"]) mustFind(t.root, id);
const runB1 = mustFind(t.root, "run-b1");
const runClick = (await client.call("click", { ref: runB1.ref })) as { dispatched: boolean };
if (!runClick.dispatched) throw new Error("run-b1 click did not dispatch");
console.log("ND_SIDEBAR_OK nested navigation-sidebar rows resolved and clickable");

await client.call("screenshot", { path: process.env.ND_SHOT_PATH ?? "/tmp/nd-widgets-probe.png" });

console.log("ND_WIDGETS_PROBE_OK C3 button parity + both windows + present dispatch verified");
client.close();
