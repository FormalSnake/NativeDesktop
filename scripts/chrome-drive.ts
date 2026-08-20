#!/usr/bin/env bun
// scripts/chrome-drive.ts [gtk|appkit]: drives examples/notes/chrome-probe.tsx
// over the automation socket via @nativedesktop/test. The probe is the window
// chrome a browser-style app builds, and every leg here is a measurement that
// was wrong on AppKit while that app was built against it:
//   1. `sidebarWidth` is a real fraction of the window, not minimumThickness.
//   2. The sourcetree fills the pane it is given.
//   3. A title-bearing widget reports its title as the snapshot's `text`.
//   4. A popover's content stays inside the frame the popover was given.
//   5. Unmounting the sidebar child removes the pane; the content pane takes
//      the whole window back, and mounting one again restores it at its width.
//   6. A collapsed pane's descendants report visible:false.
// Prints ND_CHROME_OK on success.
import { launchApp, poll, type JsonNode } from "../packages/test/src/index.ts";
import type { Backend } from "@nativedesktop/host";

const backend = (process.argv[2] as Backend | undefined) ?? "appkit";
const T = Number(process.env.ND_DRIVE_TIMEOUT_MS ?? 3000);
const WINDOW_W = 900;
const SIDEBAR_FRACTION = 0.3;

interface Rect { x: number; y: number; w: number; h: number }

function rect(node: JsonNode | null, what: string): Rect {
  if (!node) throw new Error(`${what} is not in the tree`);
  if (!node.geometry) throw new Error(`${what} has no geometry`);
  return node.geometry;
}

function findNode(root: JsonNode, testId: string): JsonNode | null {
  if (root.testID === testId) return root;
  for (const child of root.children) {
    const hit = findNode(child, testId);
    if (hit) return hit;
  }
  return null;
}

/// Every node under `root` that reports itself visible, by testID.
function visibleIds(root: JsonNode, out: string[] = []): string[] {
  if (root.testID && root.visible) out.push(root.testID);
  for (const child of root.children) visibleIds(child, out);
  return out;
}

const app = await launchApp({ entry: "examples/notes/chrome-probe.tsx", backend });
try {
  await app.waitForText("sidebar on", { timeoutMs: T });

  // ---- leg 1: sidebarWidth is a fraction of the window ---------------------
  // The pane content sits inside the pane's own insets, so the measurement is
  // "where the content pane starts", which IS the sidebar's thickness. A
  // sidebar stuck at its minimumThickness reads ~188 here whatever fraction
  // the app asked for, which is the failure this leg exists to catch.
  const wanted = WINDOW_W * SIDEBAR_FRACTION;
  const contentX = rect(await app.mustFind("sp-content"), "sp-content").x;
  if (Math.abs(contentX - wanted) > 24) {
    throw new Error(`sidebarWidth ${SIDEBAR_FRACTION} put the content pane at x=${contentX}, want ~${wanted}`);
  }
  const sidebar = rect(await app.mustFind("sp-sidebar"), "sp-sidebar");
  console.log(
    `ND_CHROME_WIDTH_OK sidebar w=${sidebar.w}, content pane starts at x=${contentX}` +
      ` (fraction ${SIDEBAR_FRACTION} of ${WINDOW_W})`,
  );

  // ---- leg 2: the sourcetree fills the pane it is given ---------------------
  const tree = rect(await app.mustFind("sp-tree"), "sp-tree");
  if (tree.w < sidebar.w - 24) throw new Error(`sp-tree w=${tree.w} does not fill the ${sidebar.w}pt sidebar`);
  console.log(`ND_CHROME_TREE_OK sourcetree w=${tree.w} in a ${sidebar.w}pt pane`);

  // ---- leg 3: schema-declared text source -----------------------------------
  // <row> has no `text`/`label` prop; docs/widgets.md declares `title` as its
  // text source, and the snapshot has to honor that on both backends.
  const row = await app.mustFind("sp-row");
  if (row.text !== "Row Title") throw new Error(`sp-row text=${JSON.stringify(row.text)}, want "Row Title"`);
  const group = await app.mustFind("sp-group");
  if (group.text !== "Group Title") throw new Error(`sp-group text=${JSON.stringify(group.text)}, want "Group Title"`);
  console.log("ND_CHROME_TEXT_OK <row>/<settingsgroup> report their title as the node's text");

  // ---- leg 4: a popover's content stays inside its frame --------------------
  await app.click("sp-panel-trigger");
  const body = await poll(
    async () => findNode((await app.tree()).root, "sp-panel-body"),
    (n) => (n?.geometry?.w ?? 0) > 0,
    { timeoutMs: T },
  );
  const panel = rect(body, "sp-panel-body");
  for (const id of ["sp-panel-heading", "sp-panel-row", "sp-panel-folder"]) {
    const child = rect(await app.mustFind(id), id);
    if (child.x < panel.x - 1 || child.x + child.w > panel.x + panel.w + 1) {
      throw new Error(
        `popover child ${id} spans ${child.x}..${child.x + child.w}, outside the panel's ${panel.x}..${panel.x + panel.w}`,
      );
    }
  }
  console.log(`ND_CHROME_POPOVER_OK panel w=${panel.w} holds every row it contains`);

  // ---- leg 5: unmounting the sidebar child removes the pane -----------------
  // GTK drops the sidebar and clears show-sidebar; AppKit kept the
  // NSSplitViewItem whatever React removed, leaving the content pane behind a
  // gutter the size of a sidebar that was no longer in the tree.
  await app.click("sp-toggle");
  await app.waitForText("sidebar off", { timeoutMs: T });
  const gone = await poll(
    async () => (await app.tree()).root,
    (root) => findNode(root, "sp-sidebar") === null,
    { timeoutMs: T },
  );
  const widened = rect(findNode(gone, "sp-content"), "sp-content");
  if (widened.x > 8) throw new Error(`content pane still starts at x=${widened.x} with no sidebar mounted`);
  console.log(`ND_CHROME_UNMOUNT_OK content pane runs ${widened.x}..${widened.x + widened.w} with the sidebar gone`);

  // ---- leg 6: mounting one again restores the pane at its width -------------
  // A second item added next to the one the unmount left behind reads as a
  // doubled gutter here, which is what AppKit did before the remove arm worked.
  await app.click("sp-toggle");
  await app.waitForText("sidebar on", { timeoutMs: T });
  const backX = (
    await poll(
      async () => rect(findNode((await app.tree()).root, "sp-content"), "sp-content"),
      (r) => r.x > 8,
      { timeoutMs: T },
    )
  ).x;
  if (Math.abs(backX - contentX) > 2) {
    throw new Error(`remounted sidebar put the content pane at x=${backX}, want ${contentX}`);
  }
  console.log(`ND_CHROME_REMOUNT_OK content pane back at x=${backX}`);
} finally {
  await app.close();
}

// ---- leg 7: a collapsed pane's descendants are not visible ------------------
// `collapsed` is create-time here, so it takes its own host: nothing else in
// the probe differs between the two runs. GTK's unmapped subtree reports false
// all the way down; AppKit hid the pane's host view and nothing under it, so a
// collapsed sidebar kept reporting visible, with the geometry it had before.
const collapsed = await launchApp({
  entry: "examples/notes/chrome-probe.tsx",
  backend,
  env: { ND_CHROME_COLLAPSED: "1", ND_APP_ID: `dev.nativedesktop.chromeCollapsed${process.pid}` },
});
try {
  await collapsed.waitForText("sidebar on", { timeoutMs: T });
  const root = (await collapsed.tree()).root;
  const visible = new Set(visibleIds(root));
  for (const id of ["sp-sidebar", "sp-new", "sp-tree"]) {
    if (!findNode(root, id)) throw new Error(`${id} is missing from the collapsed tree (it is collapsed, not unmounted)`);
    if (visible.has(id)) throw new Error(`${id} reports visible inside a collapsed pane`);
  }
  if (!visible.has("sp-content")) throw new Error("the content pane went invisible along with the collapsed sidebar");
  console.log("ND_CHROME_COLLAPSED_OK a collapsed pane's descendants report visible:false");
} finally {
  await collapsed.close();
}

console.log(`ND_CHROME_OK backend=${backend}`);
