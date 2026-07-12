#!/usr/bin/env bun
// scripts/notes-drive.ts — drives the ND Notes example over the automation
// socket. Mirrors scripts/m5c-drive.ts's AutomationClient/find scaffold.
// Proves: three-pane chrome (folders/list/editor), menu bar wiring, create
// note, type title+body, live word count, second note, search filter narrows
// the list, pin (via the header pin-button) reorders the list, delete
// (soft-delete to Trash) removes it from the All Notes scope, and File>New
// Note from the menu bar hits the same createNote() path as the header
// button.
//
// M11 SourceList Wave 3: the note-row-per-button list (list of testID
// `note-row-${id}` Button widgets, one per note) is gone — the list pane is
// now a single <sourcelist testID="note-list">. getTree reports its state as
// `itemCount` (row count) and `rows` (an ORDER-AUTHORITATIVE array of
// `{title, badge, iconName}`, core-side and backend-agnostic — see
// src/tree.zig's NodeMeta.rows / src/automation.zig's RowJson). That row
// order directly reflects what's on screen, unlike the old per-button list
// whose getTree child-array order was a HashMap iteration artifact (see the
// git history of this file for that now-obsolete workaround) — so pin/unpin
// reorder gates read `rows` directly instead of polling row y-geometry.
import { AutomationClient } from "../packages/mcp/src/socket.ts";
import { widgetMeta } from "../packages/react/src/generated/schema-meta.ts";

interface SourceListRow {
  title: string;
  badge?: string | null;
  iconName?: string | null;
}

interface TreeNode {
  ref: number;
  type: string;
  testID: string | null;
  text: string | null;
  visible: boolean;
  geometry: { x: number; y: number; w: number; h: number } | null;
  children: TreeNode[];
  itemCount?: number | null;
  rows?: SourceListRow[] | null;
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
// (e.g. confirming a note-list selection landed) instead of a follow-up action.
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
// looked up by a predicate (not a fixed testID) — kept generic since a
// widget can still report momentarily degenerate bounds for one frame after
// its container reflows (same class as setValueRetrying above), even for a
// static header control like pin-button.
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

// SourceList's row-select analog of clickRetrying: selects a row by TITLE
// substring (rather than a fixed testID/ref, since rows are recycled cells
// with no per-row ref) via setValue(list.ref, idx). idx is recomputed
// against a fresh tree on every attempt — same remount-settle race as
// setValueRetrying/clickRetrying above, plus the list's own row order can
// still be mid-reorder right after a pin/unpin or filter change.
async function selectNoteRowRetrying(titleSubstring: string, t: () => Promise<GetTreeResult>): Promise<void> {
  let lastErr: Error | null = null;
  for (let i = 0; i < 20; i++) {
    const list = mustFind(await t(), "note-list");
    const rows = list.rows ?? [];
    const idx = rows.findIndex((r) => r.title.includes(titleSubstring));
    if (idx < 0) {
      lastErr = new Error(`no note-list row matching "${titleSubstring}" (rows=${rows.map((r) => r.title).join(", ")})`);
      await new Promise((r) => setTimeout(r, 100));
      continue;
    }
    try {
      const res = (await client.call("setValue", { ref: list.ref, value: idx })) as { applied: boolean };
      if (!res.applied) throw new Error("setValue on note-list did not apply");
      return;
    } catch (e) {
      lastErr = e as Error;
      await new Promise((r) => setTimeout(r, 100));
    }
  }
  throw new Error(`selecting note-list row "${titleSubstring}" failed after retries: ${lastErr?.message}`);
}

// Polls note-list's `rows` array (order-authoritative, per the header
// comment) until `predicate` holds — a pin/unpin reorder settles a beat
// after the click ack, same remount-settle race as setValueRetrying/
// clickRetrying above.
async function waitForRowsOrder(predicate: (rows: SourceListRow[]) => boolean, t: () => Promise<GetTreeResult>): Promise<SourceListRow[]> {
  let last: SourceListRow[] = [];
  for (let i = 0; i < 20; i++) {
    const list = mustFind(await t(), "note-list");
    const rows = list.rows ?? [];
    last = rows;
    if (predicate(rows)) return rows;
    await new Promise((r) => setTimeout(r, 150));
  }
  throw new Error(`note-list row-order predicate never settled (last saw [${last.map((r) => r.title).join(", ")}])`);
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

// Native-chrome assertion (Phase A, now three-pane): the window is
// edge-to-edge and each split pane carries its OWN header bar. So
// <splitview testID="split"> (role group) wraps THREE <toolbarview> panes
// (role group) — sidebar (folders), list (search + note rows), content
// (editor) — and each pane's first child is a <headerbar> (role toolbar).
// getTree's `type` field is the schema widget name (e.g. "SplitView"/
// "ToolbarView"/"HeaderBar" — verified against src/tree.zig's putMeta, which
// stores the wire `widget_type` string unchanged); look up each node's
// accessibility role from the same generated table the codegen emits
// (packages/react/src/generated/schema-meta.ts) rather than assuming a wire
// "role" field exists.
const splitNode = mustFind(t0, "split");
const splitRole = widgetMeta[splitNode.type]?.role;
if (splitRole !== "group") throw new Error(`split node (type=${splitNode.type}) role=${splitRole}, want "group"`);

const sidebarToolbar = mustFind(t0, "sidebar-toolbar");
const listToolbar = mustFind(t0, "list-toolbar");
const contentToolbar = mustFind(t0, "content-toolbar");
for (const tv of [sidebarToolbar, listToolbar, contentToolbar]) {
  const role = widgetMeta[tv.type]?.role;
  if (role !== "group") throw new Error(`toolbarview node ${tv.testID} (type=${tv.type}) role=${role}, want "group"`);
}

const sidebarHeader = mustFind(t0, "sidebar-header");
const listHeader = mustFind(t0, "list-header");
const contentHeader = mustFind(t0, "content-header");
for (const h of [sidebarHeader, listHeader, contentHeader]) {
  const role = widgetMeta[h.type]?.role;
  if (role !== "toolbar") throw new Error(`header node ${h.testID} (type=${h.type}) role=${role}, want "toolbar"`);
}

// Each header must live inside its own toolbarview pane (not the window).
if (!find(sidebarToolbar, "sidebar-header")) throw new Error("sidebar-header is not inside sidebar-toolbar");
if (!find(listToolbar, "list-header")) throw new Error("list-header is not inside list-toolbar");
if (!find(contentToolbar, "content-header")) throw new Error("content-header is not inside content-toolbar");
// The floating editing buttons (pin/delete/new-note) live in the EDITOR
// pane's own header (end slot), not the window titlebar or the sidebar.
if (!find(contentHeader, "new-note-button")) throw new Error("new-note-button is not inside content-header");
if (!find(contentHeader, "pin-button")) throw new Error("pin-button is not inside content-header");
if (!find(contentHeader, "delete-note-button")) throw new Error("delete-note-button is not inside content-header");
console.log(
  `ND_NAVCHROME_OK edge-to-edge three-pane split with per-pane headers (split=${splitNode.type}/${splitRole}, ` +
    `sidebar-header=${sidebarHeader.type}, list-header=${listHeader.type}, content-header=${contentHeader.type})`,
);

// M13 Feature C gate: three REAL panes (sidebar/list/content), each >= 150pt
// wide, laid out left-to-right in that literal x-order — measured on the
// same widgets a user would actually click (a folder row, the note-list
// SourceList, the editor textarea), not just internal wrapper geometry.
const sidebarContent = mustFind(t0, "sidebar-content");
const listContent = mustFind(t0, "list-content");
const contentBody = mustFind(t0, "content-body");
for (const { name, node } of [
  { name: "sidebar", node: sidebarContent },
  { name: "list", node: listContent },
  { name: "content", node: contentBody },
]) {
  if (!node.geometry) throw new Error(`${name} pane has no geometry`);
  if (node.geometry.w < 150) throw new Error(`${name} pane width=${node.geometry.w} — expected >= 150`);
}
const folderRowAll0 = mustFind(t0, "folder-row-all");
const noteList0 = mustFind(t0, "note-list");
if ((noteList0.itemCount ?? 0) < 1) throw new Error("note-list has no rows at boot for ND_THREEPANE_OK");
const editorTextarea0 = mustFind(t0, "editor-textarea");
const folderX = folderRowAll0.geometry?.x;
const noteRowX = noteList0.geometry?.x;
const editorX = editorTextarea0.geometry?.x;
if (folderX == null || noteRowX == null || editorX == null) throw new Error("missing x geometry for three-pane x-order check");
if (!(folderX < noteRowX && noteRowX < editorX)) {
  throw new Error(`x-order violated: folder-row-all@${folderX}, note-list@${noteRowX}, editor-textarea@${editorX} — want folder < note < editor`);
}
console.log(
  `ND_THREEPANE_OK sidebar(w=${sidebarContent.geometry!.w})@x=${folderX} ` +
    `list(w=${listContent.geometry!.w})@x=${noteRowX} content(w=${contentBody.geometry!.w})@x=${editorX}`,
);

// Chrome geometry gates (regressed silently once, during the AppKit
// NSSplitViewController/glass migration — never again):
// 1. Pane CONTENT must clear the title bar / header area. On AppKit
//    (fullSizeContentView + unified toolbar, safe area ~52pt) and on GTK
//    (AdwHeaderBar titlebar sits above y=0 of the content coordinate space,
//    content boxes carry 8-12px padding) the first in-pane control sits
//    comfortably below y=40 only when the safe-area/header inset is intact.
// 2. The window must honor defaultWidth/Height (1100x700) — AppKit's
//    contentViewController assignment resizes the window to fitting size
//    unless the frame is explicitly preserved (measured collapse: 500x500).
const searchGeom = mustFind(t0, "search-input").geometry;
const titleGeom = mustFind(t0, "title-input").geometry;
if (!searchGeom || searchGeom.y < 40) throw new Error(`search-input y=${searchGeom?.y} — list pane content is under the titlebar (safe-area inset lost)`);
if (!titleGeom || titleGeom.y < 40) throw new Error(`title-input y=${titleGeom?.y} — content pane is under the titlebar (safe-area inset lost)`);
const rootGeom = t0.root.geometry;
if (!rootGeom || rootGeom.w < 1050) throw new Error(`window content width=${rootGeom?.w} — defaultWidth (1100) not honored`);
// 3. Each pane's content must start RIGHT of the pane before it — on AppKit
//    a pane's frame deliberately extends under the floating glass pane to
//    its left, but its content must inset past it via the safe-area leading
//    guide — an x at or left of the previous pane's right edge means content
//    is rendering underneath that pane's glass (owner-reported overlap).
if (searchGeom.x < sidebarContent.geometry!.x + sidebarContent.geometry!.w) {
  throw new Error(`search-input x=${searchGeom.x} — list pane renders under the sidebar`);
}
if (titleGeom.x < listContent.geometry!.x + listContent.geometry!.w) {
  throw new Error(`title-input x=${titleGeom.x} — content pane renders under the list pane`);
}
console.log(`ND_CHROMEGEOM_OK search-input@y=${searchGeom.y} title-input@y=${titleGeom.y},x=${titleGeom.x} root=${rootGeom.w}x${rootGeom.h}`);

let shot = (await client.call("screenshot", { path: `${shotDir}/notes-baseline.png` })) as {
  path: string;
  width: number;
  height: number;
};
if (shot.width <= 0 || shot.height <= 0) throw new Error("baseline screenshot has no dimensions");

// 1. Create a new note via the editor pane's "New note" header button.
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

// List (scoped to All Notes) now shows all three notes (2 seeded + 1 new), count label updated.
const countLabel3 = mustFind(t3, "note-count-label");
if (countLabel3.text !== "3 of 3 notes") throw new Error(`note-count-label=${countLabel3.text}, want "3 of 3 notes"`);
const list3 = mustFind(t3, "note-list");
const rows3 = list3.rows ?? [];
if ((list3.itemCount ?? -1) !== 3) throw new Error(`expected note-list itemCount=3, got ${list3.itemCount}`);
if (rows3.length !== 3) throw new Error(`expected 3 note-list rows, got ${rows3.length}`);

// Badge gate: SourceList's trailing badge is the note's live word count
// (main.tsx: `badge: wordCount(n.body) > 0 ? String(wordCount(n.body)) : undefined`)
// — "buy oat milk and coffee beans" is 6 words, matching the status-label
// wait above.
const groceryRow3 = rows3.find((r) => r.title.includes("Grocery run"));
if (!groceryRow3) throw new Error(`Grocery run row not found in note-list (rows=${rows3.map((r) => r.title).join(", ")})`);
if (groceryRow3.badge !== "6") throw new Error(`note-list badge for Grocery run=${groceryRow3.badge}, want "6"`);
console.log(`ND_BADGE_OK Grocery run row badge=${groceryRow3.badge} matches 6-word body`);

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

// 4. Search filter narrows the visible note-list rows.
const searchInput5 = mustFind(t5, "search-input");
const setSearchRes = (await client.call("setValue", { ref: searchInput5.ref, value: "grocery" })) as { applied: boolean };
if (!setSearchRes.applied) throw new Error("setValue on search-input did not apply");

await client.call("waitFor", { condition: { textContains: "1 of 4 notes" }, timeoutMs: 3000 });
let t6 = await tree();
const list6 = mustFind(t6, "note-list");
const rows6 = list6.rows ?? [];
if ((list6.itemCount ?? -1) !== 1) throw new Error(`expected note-list itemCount=1 after search filter, got ${list6.itemCount}`);
if (rows6.length !== 1) throw new Error(`expected 1 note-list row after search filter, got ${rows6.length}`);
if (!rows6[0]!.title.includes("Grocery run")) throw new Error(`filtered row title=${rows6[0]!.title}, want to include "Grocery run"`);

// Clear the search filter back to showing all notes.
const searchInput6 = mustFind(t6, "search-input");
await client.call("setValue", { ref: searchInput6.ref, value: "" });
await client.call("waitFor", { condition: { textContains: "4 of 4 notes" }, timeoutMs: 3000 });

// 5. Re-select "Grocery run" via the SourceList (selection is per-note
// state, independent of the search filter — clearing the filter does not
// change which note is selected), then pin it via the editor header's
// pin-button (Feature C: the checkbox is gone — pin/unpin now goes through
// the SAME togglePinSelected() path as the Note>Pin menu item) and assert it
// reorders first.
await selectNoteRowRetrying("Grocery run", tree);
await waitForTitle("Grocery run", tree);
await clickRetrying((t) => find(t.root, "pin-button"), tree);

// The click RPC ack only confirms the GTK/AppKit-side action was dispatched,
// which is synchronous at the signal level — the resulting onClick -> React
// state update -> NDP commitBatch round trip through the Bun child is a
// separate, later, async event. Give that at least one round trip before
// polling, same class of race as the title/word-count waits above.
await new Promise((r) => setTimeout(r, 300));

// Pin gate: note-list's `rows` array is order-authoritative (per the header
// comment), so membership + reorder + the pinned row's star icon are all one
// poll — the pinned note ("Grocery run") must be rows[0], carrying the
// leading star icon main.tsx sets for `n.pinned` (`iconName:
// "starred-symbolic"`).
const rowsAfterPin = await waitForRowsOrder(
  (rows) => rows.length === 4 && (rows[0]?.title.includes("Grocery run") ?? false) && rows[0]?.iconName === "starred-symbolic",
  tree,
);
console.log(`ND_PIN_ORDER_OK pinned row topmost: ${rowsAfterPin.map((r) => `${r.title}${r.iconName ? "*" : ""}`).join(" | ")}`);

// 5c. Unpin the same note (pin-button again — it's a toggle) and assert the
// full row-title order matches sortNotes's expected order once "Welcome to
// ND Notes" is the only pinned note again: pinned-first, then id descending
// among the rest — Welcome(1), Trip itinerary(4), Grocery run(3), Shopping
// list(2) (see examples/notes/main.tsx sortNotes). Re-select Grocery run
// first: selection is independent of pin state, but the pin-button toggles
// whichever note is currently selected.
await selectNoteRowRetrying("Grocery run", tree);
await waitForTitle("Grocery run", tree);
await clickRetrying((t) => find(t.root, "pin-button"), tree);
// Same async commit-round-trip race as the pin toggle above.
await new Promise((r) => setTimeout(r, 300));

const unpinnedOrder = ["Welcome to ND Notes", "Trip itinerary", "Grocery run", "Shopping list"];
const rowsAfterUnpin = await waitForRowsOrder(
  (rows) => rows.length === 4 && rows.map((r) => r.title).join(" | ") === unpinnedOrder.join(" | "),
  tree,
);
console.log(`ND_UNPIN_ORDER_OK order restored: ${rowsAfterUnpin.map((r) => r.title).join(" | ")}`);

// 6. Delete the selected ("Grocery run") note; Feature C makes this a soft
// delete (moves to Trash), which drops it out of the All Notes scope the
// list is currently showing — list shrinks and selection moves on exactly
// as the old hard-delete did from this view's perspective.
const t8 = await tree();
const deleteBtn8 = mustFind(t8, "delete-note-button");
const deleteRes = (await client.call("click", { ref: deleteBtn8.ref })) as { dispatched: boolean };
if (!deleteRes.dispatched) throw new Error("delete-note-button click did not dispatch");

await client.call("waitFor", { condition: { textContains: "3 of 3 notes" }, timeoutMs: 3000 });
let t9 = await tree();
const list9 = mustFind(t9, "note-list");
const rows9 = list9.rows ?? [];
if ((list9.itemCount ?? -1) !== 3) throw new Error(`expected note-list itemCount=3 after delete, got ${list9.itemCount}`);
if (rows9.length !== 3) throw new Error(`expected 3 note-list rows after delete, got ${rows9.length}`);
if (rows9.some((r) => r.title.includes("Grocery run"))) throw new Error("deleted (trashed) note still present in the All Notes list");

// 7. File > New Note from the menu bar must hit the SAME createNote() path
// as the editor header's new-note-button (Feature C: header button + menu
// item both call the same updater) — semanticClick the menuitem and confirm
// the note-list's itemCount increments exactly like button 1 did.
let t10 = await tree();
const countBefore10 = mustFind(t10, "note-list").itemCount ?? 0;
const menuNewNote = mustFind(t10, "menu-new-note");
const menuClickRes = (await client.call("click", { ref: menuNewNote.ref })) as { dispatched: boolean };
if (!menuClickRes.dispatched) throw new Error("menu-new-note click did not dispatch");
await client.call("waitFor", { condition: { textContains: "Untitled note" }, timeoutMs: 3000 });
let t11 = await tree();
const countAfter11 = mustFind(t11, "note-list").itemCount ?? 0;
if (countAfter11 !== countBefore10 + 1) {
  throw new Error(`expected note-list itemCount=${countBefore10 + 1} after File>New Note, got ${countAfter11}`);
}
const titleInput11 = mustFind(t11, "title-input");
if (titleInput11.text !== "Untitled note") throw new Error(`menu New Note did not select the new note: title-input=${titleInput11.text}`);
console.log(`ND_MENU_NEWNOTE_OK note-list itemCount ${countBefore10} -> ${countAfter11} via File>New Note menu item`);

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
