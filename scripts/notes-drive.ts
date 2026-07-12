#!/usr/bin/env bun
// scripts/notes-drive.ts — drives the ND Notes example over the automation
// socket. Mirrors scripts/m5c-drive.ts's AutomationClient/find scaffold.
// Proves: create note, type title+body, live word count, second note,
// search filter narrows the list, pin reorders the list, delete removes it.
import { AutomationClient } from "../packages/mcp/src/socket.ts";
import { widgetMeta } from "../packages/react/src/generated/schema-meta.ts";

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

function findAllPrefixed(node: TreeNode, prefix: string, acc: TreeNode[] = []): TreeNode[] {
  if (node.testID && node.testID.startsWith(prefix)) acc.push(node);
  for (const child of node.children) findAllPrefixed(child, prefix, acc);
  return acc;
}

const shotDir = process.env.ND_SHOT_DIR ?? "/tmp";
const client = await AutomationClient.connect();

async function tree(): Promise<GetTreeResult> {
  return (await client.call("getTree")) as GetTreeResult;
}

// A widget that was just (re)mounted this same commit can report a
// degenerate zero-height bound for one frame before GTK finishes layout
// (the widget is `visible: true` but not yet actionable) — same class of
// race documented for post-scroll screenshots in docs/agents/automation.md.
// Retry setValue/click a few times against a freshly-looked-up ref before
// giving up, instead of treating one -32001 as final.
async function setValueRetrying(testID: string, value: string | boolean | number, t: () => Promise<GetTreeResult>): Promise<void> {
  let lastErr: Error | null = null;
  for (let i = 0; i < 20; i++) {
    const node = find((await t()).root, testID);
    if (!node) throw new Error(`${testID} not found in tree (setValueRetrying)`);
    try {
      const res = (await client.call("setValue", { ref: node.ref, value })) as { applied: boolean };
      if (!res.applied) throw new Error(`setValue on ${testID} did not apply`);
      return;
    } catch (e) {
      lastErr = e as Error;
      await new Promise((r) => setTimeout(r, 100));
    }
  }
  throw new Error(`setValue on ${testID} failed after retries: ${lastErr?.message}`);
}

// Same remount-settle race as setValueRetrying, for a plain read-and-assert
// (e.g. confirming a note-row click landed) instead of a follow-up action.
async function waitForTitle(expected: string, t: () => Promise<GetTreeResult>): Promise<TreeNode> {
  let lastActual = "";
  for (let i = 0; i < 20; i++) {
    const node = find((await t()).root, "title-input");
    if (node?.text === expected) return node;
    lastActual = node?.text ?? "<missing>";
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`title-input never settled to "${expected}" (last saw "${lastActual}")`);
}

// Same remount-settle race as setValueRetrying, for a click against a node
// looked up by a predicate (not a fixed testID) whose bounds can be
// momentarily degenerate right after its container reflows it back into
// view (observed live on the Mac AppKit backend: a row re-entering an
// NSStackView after a search filter clears settles its bounds a beat after
// the tree/label already reports it present).
async function clickRetrying(findNode: (t: GetTreeResult) => TreeNode | null, t: () => Promise<GetTreeResult>): Promise<void> {
  let lastErr: Error | null = null;
  for (let i = 0; i < 20; i++) {
    const node = findNode(await t());
    if (!node) throw new Error("clickRetrying: node not found");
    try {
      const res = (await client.call("click", { ref: node.ref })) as { dispatched: boolean };
      if (!res.dispatched) throw new Error("click did not dispatch");
      return;
    } catch (e) {
      lastErr = e as Error;
      await new Promise((r) => setTimeout(r, 100));
    }
  }
  throw new Error(`click failed after retries: ${lastErr?.message}`);
}

function mustFind(t: GetTreeResult, testID: string): TreeNode {
  const n = find(t.root, testID);
  if (!n) throw new Error(`${testID} not found in tree`);
  return n;
}

// Row bounds' y-coordinates ARE a reliable order signal, unlike getTree's
// child-ARRAY order (see the big comment below on hashmap iteration order) —
// a pin/unpin reorder must eventually settle into the sort examples/notes/
// main.tsx's sortNotes applies (pinned-first, then id descending). Poll for
// that y-order instead of trusting a single snapshot, same remount-settle
// race as setValueRetrying/clickRetrying above.
async function waitForRowsByY(
  predicate: (rows: { id: string; y: number }[]) => boolean,
  t: () => Promise<GetTreeResult>,
): Promise<{ id: string; y: number }[]> {
  let last: { id: string; y: number }[] = [];
  for (let i = 0; i < 20; i++) {
    const rows = findAllPrefixed((await t()).root, "note-row-")
      .map((r) => ({ id: r.testID!, y: r.geometry?.y ?? -1 }))
      .sort((a, b) => a.y - b.y);
    last = rows;
    if (predicate(rows)) return rows;
    await new Promise((r) => setTimeout(r, 150));
  }
  throw new Error(`row y-order predicate never settled (last saw [${last.map((r) => `${r.id}@${r.y.toFixed(1)}`).join(", ")}])`);
}

// 0. Window present; baseline screenshot with the seeded first note selected.
const waitedWindow = (await client.call("waitFor", {
  condition: { textContains: "ND Notes" },
  timeoutMs: 3000,
})) as { matched: boolean };
if (!waitedWindow.matched) throw new Error("waitFor window title did not match");

let t0 = await tree();
const titleInput0 = mustFind(t0, "title-input");
if (titleInput0.text !== "Welcome to ND Notes") throw new Error(`unexpected seeded title: ${titleInput0.text}`);

// Native-chrome assertion (Phase A): the window is edge-to-edge and each
// split pane carries its OWN header bar. So <splitview testID="split">
// (role group) wraps two <toolbarview> panes (role group), and each pane's
// first child is a <headerbar> (role toolbar) — the sidebar header holds the
// "New Note" button, the content header shows the note title. getTree's
// `type` field is the schema widget name (e.g. "SplitView"/"ToolbarView"/
// "HeaderBar" — verified against src/tree.zig's putMeta, which stores the
// wire `widget_type` string unchanged); look up each node's accessibility
// role from the same generated table the codegen emits (packages/react/src/
// generated/schema-meta.ts) rather than assuming a wire "role" field exists.
const splitNode = mustFind(t0, "split");
const splitRole = widgetMeta[splitNode.type]?.role;
if (splitRole !== "group") throw new Error(`split node (type=${splitNode.type}) role=${splitRole}, want "group"`);

const sidebarToolbar = mustFind(t0, "sidebar-toolbar");
const contentToolbar = mustFind(t0, "content-toolbar");
for (const tv of [sidebarToolbar, contentToolbar]) {
  const role = widgetMeta[tv.type]?.role;
  if (role !== "group") throw new Error(`toolbarview node ${tv.testID} (type=${tv.type}) role=${role}, want "group"`);
}

const sidebarHeader = mustFind(t0, "sidebar-header");
const contentHeader = mustFind(t0, "content-header");
for (const h of [sidebarHeader, contentHeader]) {
  const role = widgetMeta[h.type]?.role;
  if (role !== "toolbar") throw new Error(`header node ${h.testID} (type=${h.type}) role=${role}, want "toolbar"`);
}

// Each header must live inside its own toolbarview pane (not the window).
if (!find(sidebarToolbar, "sidebar-header")) throw new Error("sidebar-header is not inside sidebar-toolbar");
if (!find(contentToolbar, "content-header")) throw new Error("content-header is not inside content-toolbar");
// The "New Note" button lives in the sidebar's header, not the window titlebar.
if (!find(sidebarHeader, "new-note-button")) throw new Error("new-note-button is not inside sidebar-header");
console.log(
  `ND_NAVCHROME_OK edge-to-edge split with per-pane headers (split=${splitNode.type}/${splitRole}, ` +
    `sidebar-header=${sidebarHeader.type}, content-header=${contentHeader.type})`,
);

// Chrome geometry gates (regressed silently once, during the AppKit
// NSSplitViewController/glass migration — never again):
// 1. Pane CONTENT must clear the title bar / header area. On AppKit
//    (fullSizeContentView + unified toolbar, safe area ~52pt) and on GTK
//    (AdwHeaderBar titlebar sits above y=0 of the content coordinate space,
//    content boxes carry 10-12px padding) the first in-pane control sits
//    comfortably below y=40 only when the safe-area/header inset is intact.
// 2. The window must honor defaultWidth/Height (900x600) — AppKit's
//    contentViewController assignment resizes the window to fitting size
//    unless the frame is explicitly preserved (measured collapse: 500x500).
const searchGeom = mustFind(t0, "search-input").geometry;
const titleGeom = mustFind(t0, "title-input").geometry;
if (!searchGeom || searchGeom.y < 40) throw new Error(`search-input y=${searchGeom?.y} — sidebar content is under the titlebar (safe-area inset lost)`);
if (!titleGeom || titleGeom.y < 40) throw new Error(`title-input y=${titleGeom?.y} — content pane is under the titlebar (safe-area inset lost)`);
const rootGeom = t0.root.geometry;
if (!rootGeom || rootGeom.w < 850) throw new Error(`window content width=${rootGeom?.w} — defaultWidth (900) not honored`);
// 3. Content-pane layout must start RIGHT of the sidebar (sidebar fraction
//    0.28 * 900 = 252). On AppKit the pane's frame deliberately extends
//    under the floating glass sidebar, but its content must inset past it
//    via the safe-area leading guide — an x below the sidebar width means
//    the editor is rendering underneath the glass (owner-reported overlap).
if (titleGeom.x < 200) throw new Error(`title-input x=${titleGeom.x} — content pane renders under the sidebar`);
console.log(`ND_CHROMEGEOM_OK search-input@y=${searchGeom.y} title-input@y=${titleGeom.y},x=${titleGeom.x} root=${rootGeom.w}x${rootGeom.h}`);

let shot = (await client.call("screenshot", { path: `${shotDir}/notes-baseline.png` })) as {
  path: string;
  width: number;
  height: number;
};
if (shot.width <= 0 || shot.height <= 0) throw new Error("baseline screenshot has no dimensions");

// 1. Create a new note via the "New note" button.
const newNoteBtn = mustFind(t0, "new-note-button");
const createRes = (await client.call("click", { ref: newNoteBtn.ref })) as { dispatched: boolean };
if (!createRes.dispatched) throw new Error("new-note-button click did not dispatch");

const waitedNewNote = (await client.call("waitFor", {
  condition: { textContains: "Untitled note" },
  timeoutMs: 3000,
})) as { matched: boolean };
if (!waitedNewNote.matched) throw new Error("waitFor Untitled note did not match");

let t1 = await tree();
const titleInput1 = mustFind(t1, "title-input");
if (titleInput1.text !== "Untitled note") throw new Error(`new note title wrong: ${titleInput1.text}`);

// 2. Type into title + editor (setValue replaces full text for TextInput/TextArea).
await setValueRetrying("title-input", "Grocery run", tree);
await client.call("waitFor", { condition: { textContains: "Grocery run" }, timeoutMs: 3000 });

await setValueRetrying("editor-textarea", "buy oat milk and coffee beans", tree);

const waitedWords = (await client.call("waitFor", {
  condition: { textContains: "6 words" },
  timeoutMs: 3000,
})) as { matched: boolean };
if (!waitedWords.matched) throw new Error("waitFor 6-word status label did not match");

let t3 = await tree();
const titleInput3 = mustFind(t3, "title-input");
if (titleInput3.text !== "Grocery run") throw new Error(`title-input text=${titleInput3.text}, want "Grocery run"`);
const editor3 = mustFind(t3, "editor-textarea");
if (editor3.text !== "buy oat milk and coffee beans") throw new Error(`editor-textarea text=${editor3.text}`);
const status3 = mustFind(t3, "status-label");
if (!status3.text?.startsWith("6 words")) throw new Error(`status-label=${status3.text}, want to start with "6 words"`);
if (!status3.text?.includes("Saved")) throw new Error(`status-label=${status3.text}, want to include "Saved"`);

// List now shows all three notes (2 seeded + 1 new), count label updated.
const countLabel3 = mustFind(t3, "note-count-label");
if (countLabel3.text !== "3 of 3 notes") throw new Error(`note-count-label=${countLabel3.text}, want "3 of 3 notes"`);
const rows3 = findAllPrefixed(t3.root, "note-row-");
if (rows3.length !== 3) throw new Error(`expected 3 note rows, got ${rows3.length}`);

// 3. Create a fourth note distinguishable by title, to test search + pin ordering.
const newNoteBtn3 = mustFind(t3, "new-note-button");
await client.call("click", { ref: newNoteBtn3.ref });
await client.call("waitFor", { condition: { textContains: "4 of 4 notes" }, timeoutMs: 3000 });
let t4 = await tree();
const titleInput4 = mustFind(t4, "title-input");
if (titleInput4.text !== "Untitled note") throw new Error(`second new note title wrong: ${titleInput4.text}`);
await setValueRetrying("title-input", "Trip itinerary", tree);
await client.call("waitFor", { condition: { textContains: "Trip itinerary" }, timeoutMs: 3000 });

let t5 = await tree();
const countLabel5 = mustFind(t5, "note-count-label");
if (countLabel5.text !== "4 of 4 notes") throw new Error(`note-count-label=${countLabel5.text}, want "4 of 4 notes"`);

// 4. Search filter narrows the visible note-row list.
const searchInput5 = mustFind(t5, "search-input");
const setSearchRes = (await client.call("setValue", { ref: searchInput5.ref, value: "grocery" })) as { applied: boolean };
if (!setSearchRes.applied) throw new Error("setValue on search-input did not apply");

await client.call("waitFor", { condition: { textContains: "1 of 4 notes" }, timeoutMs: 3000 });
let t6 = await tree();
const rows6 = findAllPrefixed(t6.root, "note-row-");
if (rows6.length !== 1) throw new Error(`expected 1 note row after search filter, got ${rows6.length}`);
if (!rows6[0]!.text?.includes("Grocery run")) throw new Error(`filtered row text=${rows6[0]!.text}, want to include "Grocery run"`);

// Clear the search filter back to showing all notes.
const searchInput6 = mustFind(t6, "search-input");
await client.call("setValue", { ref: searchInput6.ref, value: "" });
await client.call("waitFor", { condition: { textContains: "4 of 4 notes" }, timeoutMs: 3000 });

// 5. Re-select "Grocery run" via its note-row button (selection is per-note
// state, independent of the search filter — clearing the filter does not
// change which note is selected), then pin it and assert it reorders first.
const findGroceryRow = (t: GetTreeResult): TreeNode | null =>
  findAllPrefixed(t.root, "note-row-").find((r) => r.text?.includes("Grocery run")) ?? null;
if (!findGroceryRow(await tree())) throw new Error("Grocery run row not found after clearing search");
await clickRetrying(findGroceryRow, tree);
await waitForTitle("Grocery run", tree);
await setValueRetrying("pin-checkbox", true, tree);

// setValue's RPC ack only confirms the GTK-side action was dispatched
// (gtk.CheckButton.setActive), which is synchronous at the signal level —
// the resulting onToggled -> React state update -> NDP commitBatch round
// trip through the Bun child is a separate, later, async event. Give that
// at least one round trip before polling, same class of race as the
// title/word-count waits above.
await new Promise((r) => setTimeout(r, 300));

// getTree's reported child ORDER is not a reliable signal for asserting
// sort/reorder behavior: src/automation.zig's handleGetTree builds each
// parent's children list by iterating `tree.meta` (an
// AutoHashMapUnmanaged) and grouping by parent id — Zig HashMap iteration
// order reflects bucket/hash layout, not node-creation or GTK-sibling
// order, despite the "insertion order" comment at that call site. Verified
// live: an NDP wire trace of this exact pin action shows the host correctly
// emitting create+append ops in the right final sequence
// (note-row-3,1,4,2 = Grocery,Welcome,Trip,Shopping, pinned-first then id
// descending) AND the checkbox's `checked:true` update landing — but the
// SAME getTree call immediately after can report the children back in
// plain id order (1,2,3,4). The app and the widget tree are both correct;
// only the automation reflection of "what order are they in" is unreliable
// for a reordering list. Assert what getTree DOES guarantee instead:
// membership (all 4 rows present, by testID, order-independent) and the
// checkbox's own semantic effect (still checked) — then let the final
// screenshot be the visual proof of the actual on-screen order/accent.
let rows8: TreeNode[] = [];
let t8: GetTreeResult = await tree();
for (let i = 0; i < 20; i++) {
  t8 = await tree();
  rows8 = findAllPrefixed(t8.root, "note-row-");
  if (rows8.length === 4) break;
  await new Promise((r) => setTimeout(r, 150));
}
if (rows8.length !== 4) throw new Error(`expected 4 note rows, got ${rows8.length}`);
const rowIds = new Set(rows8.map((r) => r.testID));
for (const id of ["note-row-1", "note-row-2", "note-row-3", "note-row-4"]) {
  if (!rowIds.has(id)) throw new Error(`expected ${id} present after pin, rows=${rows8.map((r) => `${r.testID}:${r.text}`).join(" | ")}`);
}
const groceryRowAfterPin = rows8.find((r) => r.testID === "note-row-3");
if (!groceryRowAfterPin?.text?.includes("Grocery run")) throw new Error(`note-row-3 label wrong after pin: ${groceryRowAfterPin?.text}`);

// 5b. Bug 1 regression gate: bounds ARE a reliable order signal (unlike
// getTree's child-array order, per the comment above) — assert the pinned
// row is now visually on top, i.e. its y is strictly the smallest among all
// note-row-* rows (rows8[0] after sorting by y is the topmost row).
const rowsAfterPin = await waitForRowsByY((rows) => rows.length === 4 && rows[0]!.id === "note-row-3" && rows[0]!.y < rows[1]!.y, tree);
console.log(`ND_PIN_YORDER_OK pinned row y-topmost: ${rowsAfterPin.map((r) => `${r.id}@${r.y.toFixed(1)}`).join(", ")}`);

// 5c. Unpin the same note and assert the full y-order matches sortNotes's
// expected order once note 1 (Welcome) is the only pinned note again:
// pinned-first (1), then id descending among the unpinned (4,3,2) — i.e.
// note-row-1, note-row-4, note-row-3, note-row-2 (see examples/notes/
// main.tsx sortNotes). Re-select note-row-3 first: selection is independent
// of pin state, but the pin-checkbox only renders for the selected row.
await clickRetrying((t) => find(t.root, "note-row-3"), tree);
await waitForTitle("Grocery run", tree);
await setValueRetrying("pin-checkbox", false, tree);
// Same async commit-round-trip race as the pin toggle above.
await new Promise((r) => setTimeout(r, 300));

const unpinnedOrder = ["note-row-1", "note-row-4", "note-row-3", "note-row-2"];
const rowsAfterUnpin = await waitForRowsByY(
  (rows) => rows.length === 4 && rows.map((r) => r.id).join(",") === unpinnedOrder.join(","),
  tree,
);
console.log(`ND_UNPIN_YORDER_OK y-order restored: ${rowsAfterUnpin.map((r) => `${r.id}@${r.y.toFixed(1)}`).join(", ")}`);

// Refresh t8 (used below to find delete-note-button) to the post-unpin tree.
t8 = await tree();

// 6. Delete the selected ("Grocery run") note; list shrinks and selection moves on.
const deleteBtn8 = mustFind(t8, "delete-note-button");
const deleteRes = (await client.call("click", { ref: deleteBtn8.ref })) as { dispatched: boolean };
if (!deleteRes.dispatched) throw new Error("delete-note-button click did not dispatch");

await client.call("waitFor", { condition: { textContains: "3 of 3 notes" }, timeoutMs: 3000 });
let t9 = await tree();
const rows9 = findAllPrefixed(t9.root, "note-row-");
if (rows9.length !== 3) throw new Error(`expected 3 note rows after delete, got ${rows9.length}`);
if (rows9.some((r) => r.text?.includes("Grocery run"))) throw new Error("deleted note still present in list");

// Final screenshot, polling like m5c-drive.ts (post-mutation frame can race invalidation).
let finalShot: { path: string; width: number; height: number } | null = null;
let lastErr: Error | null = null;
for (let i = 0; i < 20; i++) {
  try {
    finalShot = (await client.call("screenshot", { path: `${shotDir}/notes-final.png` })) as {
      path: string;
      width: number;
      height: number;
    };
    break;
  } catch (e) {
    lastErr = e as Error;
    await new Promise((r) => setTimeout(r, 150));
  }
}
if (!finalShot) throw new Error(`final screenshot failed after retries: ${lastErr?.message}`);
if (finalShot.width <= 0 || finalShot.height <= 0) throw new Error("final screenshot has no dimensions");

console.log(
  `ND_NOTES_OK create+edit+search+pin+delete round-trip verified baseline=${shot.path} final=${finalShot.path} ${finalShot.width}x${finalShot.height}`,
);
client.close();
