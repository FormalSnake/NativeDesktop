#!/usr/bin/env bun
// scripts/threepane-drive.ts — drives examples/notes/threepane-probe.tsx over
// the automation socket (mirrors scripts/notes-drive.ts's AutomationClient/
// find scaffold). Proves the M13 Feature B three-pane SplitView: all three
// panes (sidebar/list/content) are present, laid out left-to-right in that
// order, and each is wide enough to be usable.
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

function mustFind(t: GetTreeResult, testID: string): TreeNode {
  const n = find(t.root, testID);
  if (!n) throw new Error(`${testID} not found in tree`);
  return n;
}

const client = await AutomationClient.connect();

async function tree(): Promise<GetTreeResult> {
  return (await client.call("getTree")) as GetTreeResult;
}

// Window titles aren't captured as node text (tree.zig's create-op meta only
// keys off literal "text"/"label" props, which "title" is not) — wait on the
// sidebar row's Label text instead, same idiom notes-drive.ts relies on
// (its "ND Notes" wait actually matches the seeded note title text, not the
// window's title either).
const waitedWindow = (await client.call("waitFor", {
  condition: { textContains: "All Notes" },
  timeoutMs: 3000,
})) as { matched: boolean };
if (!waitedWindow.matched) throw new Error("waitFor sidebar row did not match");

const t = await tree();

// All three panes present (toolbarview + headerbar + labeled row each).
mustFind(t, "probe-sidebar-toolbar");
mustFind(t, "probe-list-toolbar");
mustFind(t, "probe-content-toolbar");
mustFind(t, "probe-sidebar-header");
mustFind(t, "probe-list-header");
mustFind(t, "probe-content-header");
mustFind(t, "probe-sidebar-row");
mustFind(t, "probe-list-row");
mustFind(t, "probe-content-row");

// x-order/width are measured on each pane's CONTENT box, not the
// <toolbarview> pane node itself: on mac the pane (NDToolbarPaneView) is a
// logical holder that never enters the real view hierarchy (same for
// <headerbar> — see swift/Sources/NDShell/HeaderBar.swift), so it reports
// degenerate zero geometry there by design. GTK's AdwToolbarView IS a real
// widget with real geometry, but measuring the content box keeps the
// assertion meaningful and platform-consistent on both backends.
const sidebarContent = mustFind(t, "probe-sidebar-content");
const listContent = mustFind(t, "probe-list-content");
const contentContent = mustFind(t, "probe-content-content");

const panes = [
  { name: "sidebar", node: sidebarContent },
  { name: "list", node: listContent },
  { name: "content", node: contentContent },
];

for (const { name, node } of panes) {
  if (!node.geometry) throw new Error(`${name} pane has no geometry`);
  if (node.geometry.w < 150) throw new Error(`${name} pane width=${node.geometry.w} — expected >= 150`);
}

const sidebarX = sidebarContent.geometry!.x;
const listX = listContent.geometry!.x;
const contentX = contentContent.geometry!.x;
if (!(sidebarX < listX && listX < contentX)) {
  throw new Error(`x-order violated: sidebar@${sidebarX}, list@${listX}, content@${contentX} — want sidebar < list < content`);
}

console.log(
  `ND_THREEPANE_PROBE_OK sidebar@x=${sidebarX},w=${sidebarContent.geometry!.w} ` +
    `list@x=${listX},w=${listContent.geometry!.w} content@x=${contentX},w=${contentContent.geometry!.w}`,
);
client.close();
