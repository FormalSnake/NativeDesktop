#!/usr/bin/env bun
// scripts/threepane-drive.ts: drives examples/notes/threepane-probe.tsx over
// the automation socket (mirrors scripts/notes-drive.ts's locator scaffold).
// Proves the M13 Feature B three-pane SplitView: all three panes
// (sidebar/list/content) are present, laid out left-to-right in that order,
// and each is wide enough to be usable.
import { connectApp, expect } from "../packages/test/src/index.ts";

const app = await connectApp();

// Waits on the sidebar row's Label text, the idiom notes-drive.ts uses too
// (its "ND Notes" wait matches the seeded note title, not the window's title).
await app.waitForText("All Notes", { timeoutMs: 3000 });

// All three panes present (toolbarview + headerbar + labeled row each).
for (const id of [
  "probe-sidebar-toolbar",
  "probe-list-toolbar",
  "probe-content-toolbar",
  "probe-sidebar-header",
  "probe-list-header",
  "probe-content-header",
  "probe-sidebar-row",
  "probe-list-row",
  "probe-content-row",
]) {
  await expect(app.getByTestId(id)).toBeAttached();
}

// x-order/width are measured on each pane's CONTENT box, not the
// <toolbarview> pane node itself: on mac the pane (NDToolbarPaneView) is a
// logical holder that never enters the real view hierarchy (same for
// <headerbar>, see swift/Sources/NDShell/HeaderBar.swift), so it reports
// degenerate zero geometry there by design. GTK's AdwToolbarView IS a real
// widget with real geometry, but measuring the content box keeps the
// assertion meaningful and platform-consistent on both backends.
const panes = [
  { name: "sidebar", node: app.getByTestId("probe-sidebar-content") },
  { name: "list", node: app.getByTestId("probe-list-content") },
  { name: "content", node: app.getByTestId("probe-content-content") },
];

const boxes: Record<string, { x: number; y: number; width: number; height: number }> = {};
for (const { name, node } of panes) {
  const box = await node.boundingBox();
  if (!box) throw new Error(`${name} pane has no geometry`);
  if (box.width < 150) throw new Error(`${name} pane width=${box.width}, expected >= 150`);
  boxes[name] = box;
}

const sidebarX = boxes.sidebar!.x;
const listX = boxes.list!.x;
const contentX = boxes.content!.x;
if (!(sidebarX < listX && listX < contentX)) {
  throw new Error(`x-order violated: sidebar@${sidebarX}, list@${listX}, content@${contentX}, want sidebar < list < content`);
}

console.log(
  `ND_THREEPANE_PROBE_OK sidebar@x=${sidebarX},w=${boxes.sidebar!.width} ` +
    `list@x=${listX},w=${boxes.list!.width} content@x=${contentX},w=${boxes.content!.width}`,
);
await app.close();
