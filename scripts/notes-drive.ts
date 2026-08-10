#!/usr/bin/env bun
// scripts/notes-drive.ts [backend] — drives the ND Notes example over
// @nativedesktop/test. Proves: three-pane chrome (folders/list/editor), menu
// bar wiring, create note, type title+body, live word count, second note,
// search filter narrows the list, pin (via the header pin-button) reorders
// the list, delete (soft-delete to Trash) removes it from the All Notes
// scope, and File>New Note from the menu bar hits the same createNote() path
// as the header button.
//
// M11 SourceList Wave 3: the note-row-per-button list is gone — the list
// pane is a single <sourcelist testID="note-list">. getTree reports its
// state as `itemCount` (row count) and `rows` (an ORDER-AUTHORITATIVE array
// of {title, badge, iconName}), so pin/unpin reorder gates read `rows`
// directly instead of polling row y-geometry.
//
// This used to hand-roll setValueRetrying/clickRetrying/selectNoteRowRetrying
// wrappers around every action to route around a widget-just-mounted
// geometry race. That race is fixed at the host level this wave (the
// vtCreate ref_sink + release_node crash fix, §4a of the automation design) —
// actions are now single-RPC, host-side-resolved, no client retry loop. The
// one genuine settle-wait that remains (a pin/unpin reorder landing a beat
// after the click ack, a real async NDP round trip) uses poll().
import { launchApp, findNode, poll } from "../packages/test/src/index.ts";
import type { Backend } from "@nativedesktop/host";
import { widgetMeta } from "../packages/react/src/generated/schema-meta.ts";

interface SourceListRow {
  title: string;
  badge?: string | null;
  iconName?: string | null;
}

const backend = process.argv[2] as Backend | undefined;
const shotDir = process.env.ND_SHOT_DIR ?? "/tmp";

const app = await launchApp({ entry: "examples/notes/main.tsx", backend });

// Polls note-list's `rows` array (order-authoritative, per the header
// comment) until `predicate` holds — a pin/unpin reorder settles a beat
// after the click ack: real async commit-round-trip latency, not a widget
// race, so this stays a poll rather than collapsing to a single read.
async function waitForRowsOrder(predicate: (rows: SourceListRow[]) => boolean): Promise<SourceListRow[]> {
  const list = await poll(
    () => app.mustFind("note-list"),
    (n) => predicate(n.rows ?? []),
    { timeoutMs: 3000, intervalMs: 150 },
  ).catch((e: Error) => {
    throw new Error(`note-list row-order predicate never settled: ${e.message}`);
  });
  return list.rows ?? [];
}

// SourceList's row-select analog of click-by-testID: rows are recycled cells
// with no per-row ref, so selection goes through setValue(list.ref, idx) with
// idx resolved by title substring against the current `rows` snapshot.
async function selectNoteRow(titleSubstring: string): Promise<void> {
  const list = await app.mustFind("note-list");
  const rows = list.rows ?? [];
  const idx = rows.findIndex((r) => r.title.includes(titleSubstring));
  if (idx < 0) throw new Error(`no note-list row matching "${titleSubstring}" (rows=${rows.map((r) => r.title).join(", ")})`);
  const res = await app.setValue("note-list", idx);
  if (!res.applied) throw new Error("setValue on note-list did not apply");
}

// 0. Window present; baseline screenshot with the seeded first note selected.
const waitedWindow = await app.waitForText("ND Notes", { timeoutMs: 3000 });
if (!waitedWindow.matched) throw new Error("waitFor window title did not match");

const titleInput0 = await app.mustFind("title-input");
if (titleInput0.text !== "Welcome to ND Notes") throw new Error(`unexpected seeded title: ${titleInput0.text}`);

// Native-chrome assertion (Phase A, now three-pane): the window is
// edge-to-edge and each split pane carries its OWN header bar. So
// <splitview testID="split"> (role group) wraps THREE <toolbarview> panes
// (role group) — sidebar (folders), list (search + note rows), content
// (editor) — and each pane's first child is a <headerbar> (role toolbar).
// getTree's `type` field is the schema widget name; look up each node's
// accessibility role from the same generated table the codegen emits
// (packages/react/src/generated/schema-meta.ts) rather than assuming a wire
// "role" field exists.
const splitNode = await app.mustFind("split");
const splitRole = widgetMeta[splitNode.type]?.role;
if (splitRole !== "group") throw new Error(`split node (type=${splitNode.type}) role=${splitRole}, want "group"`);

const sidebarToolbar = await app.mustFind("sidebar-toolbar");
const listToolbar = await app.mustFind("list-toolbar");
const contentToolbar = await app.mustFind("content-toolbar");
for (const tv of [sidebarToolbar, listToolbar, contentToolbar]) {
  const role = widgetMeta[tv.type]?.role;
  if (role !== "group") throw new Error(`toolbarview node ${tv.testID} (type=${tv.type}) role=${role}, want "group"`);
}

const sidebarHeader = await app.mustFind("sidebar-header");
const listHeader = await app.mustFind("list-header");
const contentHeader = await app.mustFind("content-header");
for (const h of [sidebarHeader, listHeader, contentHeader]) {
  const role = widgetMeta[h.type]?.role;
  if (role !== "toolbar") throw new Error(`header node ${h.testID} (type=${h.type}) role=${role}, want "toolbar"`);
}

// Each header must live inside its own toolbarview pane (not the window) —
// a scoped subtree search, not a whole-tree find.
if (!findNode(sidebarToolbar, "sidebar-header")) throw new Error("sidebar-header is not inside sidebar-toolbar");
if (!findNode(listToolbar, "list-header")) throw new Error("list-header is not inside list-toolbar");
if (!findNode(contentToolbar, "content-header")) throw new Error("content-header is not inside content-toolbar");
// The floating editing buttons (pin/delete/new-note) live in the EDITOR
// pane's own header (end slot), not the window titlebar or the sidebar.
if (!findNode(contentHeader, "new-note-button")) throw new Error("new-note-button is not inside content-header");
if (!findNode(contentHeader, "pin-button")) throw new Error("pin-button is not inside content-header");
if (!findNode(contentHeader, "delete-note-button")) throw new Error("delete-note-button is not inside content-header");
console.log(
  `ND_NAVCHROME_OK edge-to-edge three-pane split with per-pane headers (split=${splitNode.type}/${splitRole}, ` +
    `sidebar-header=${sidebarHeader.type}, list-header=${listHeader.type}, content-header=${contentHeader.type})`,
);

// M13 Feature C gate: three REAL panes (sidebar/list/content), each >= 150pt
// wide, laid out left-to-right in that literal x-order — measured on the
// same widgets a user would actually click.
const sidebarContent = await app.mustFind("sidebar-content");
const listContent = await app.mustFind("list-content");
const contentBody = await app.mustFind("content-body");
for (const { name, node } of [
  { name: "sidebar", node: sidebarContent },
  { name: "list", node: listContent },
  { name: "content", node: contentBody },
]) {
  if (!node.geometry) throw new Error(`${name} pane has no geometry`);
  if (node.geometry.w < 150) throw new Error(`${name} pane width=${node.geometry.w} — expected >= 150`);
}
const folderRowAll0 = await app.mustFind("folder-row-all");
const noteList0 = await app.mustFind("note-list");
if ((noteList0.itemCount ?? 0) < 1) throw new Error("note-list has no rows at boot for ND_THREEPANE_OK");
const editorTextarea0 = await app.mustFind("editor-textarea");
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
// 1. Pane CONTENT must clear the title bar / header area.
// 2. The window must honor defaultWidth/Height (1100x700).
const searchGeom = (await app.mustFind("search-input")).geometry;
const titleGeom = (await app.mustFind("title-input")).geometry;
if (!searchGeom || searchGeom.y < 40) throw new Error(`search-input y=${searchGeom?.y} — list pane content is under the titlebar (safe-area inset lost)`);
if (!titleGeom || titleGeom.y < 40) throw new Error(`title-input y=${titleGeom?.y} — content pane is under the titlebar (safe-area inset lost)`);
const rootTree = await app.tree();
const rootGeom = rootTree.root.geometry;
if (!rootGeom || rootGeom.w < 1050) throw new Error(`window content width=${rootGeom?.w} — defaultWidth (1100) not honored`);
// 3. Each pane's content must start RIGHT of the pane before it.
if (searchGeom.x < sidebarContent.geometry!.x + sidebarContent.geometry!.w) {
  throw new Error(`search-input x=${searchGeom.x} — list pane renders under the sidebar`);
}
if (titleGeom.x < listContent.geometry!.x + listContent.geometry!.w) {
  throw new Error(`title-input x=${titleGeom.x} — content pane renders under the list pane`);
}
console.log(`ND_CHROMEGEOM_OK search-input@y=${searchGeom.y} title-input@y=${titleGeom.y},x=${titleGeom.x} root=${rootGeom.w}x${rootGeom.h}`);

const shot = await app.screenshot(`${shotDir}/notes-baseline.png`);

// 1. Create a new note via the editor pane's "New note" header button.
const createRes = await app.click("new-note-button");
if (!createRes.dispatched) throw new Error("new-note-button click did not dispatch");

const waitedNewNote = await app.waitForText("Untitled note", { timeoutMs: 3000 });
if (!waitedNewNote.matched) throw new Error("waitFor Untitled note did not match");

const titleInput1 = await app.mustFind("title-input");
if (titleInput1.text !== "Untitled note") throw new Error(`new note title wrong: ${titleInput1.text}`);

// 2. Type into title + editor (setValue replaces full text for TextInput/TextArea).
await app.setValue("title-input", "Grocery run");
await app.waitForText("Grocery run", { timeoutMs: 3000 });

await app.setValue("editor-textarea", "buy oat milk and coffee beans");

const waitedWords = await app.waitForText("6 words", { timeoutMs: 3000 });
if (!waitedWords.matched) throw new Error("waitFor 6-word status label did not match");

const titleInput3 = await app.mustFind("title-input");
if (titleInput3.text !== "Grocery run") throw new Error(`title-input text=${titleInput3.text}, want "Grocery run"`);
const editor3 = await app.mustFind("editor-textarea");
if (editor3.text !== "buy oat milk and coffee beans") throw new Error(`editor-textarea text=${editor3.text}`);
const status3 = await app.mustFind("status-label");
if (!status3.text?.startsWith("6 words")) throw new Error(`status-label=${status3.text}, want to start with "6 words"`);
if (!status3.text?.includes("Saved")) throw new Error(`status-label=${status3.text}, want to include "Saved"`);

// List (scoped to All Notes) now shows all three notes (2 seeded + 1 new), count label updated.
const countLabel3 = await app.mustFind("note-count-label");
if (countLabel3.text !== "3 of 3 notes") throw new Error(`note-count-label=${countLabel3.text}, want "3 of 3 notes"`);
const list3 = await app.mustFind("note-list");
const rows3 = list3.rows ?? [];
if ((list3.itemCount ?? -1) !== 3) throw new Error(`expected note-list itemCount=3, got ${list3.itemCount}`);
if (rows3.length !== 3) throw new Error(`expected 3 note-list rows, got ${rows3.length}`);

// Badge gate: SourceList's trailing badge is the note's live word count
// ("buy oat milk and coffee beans" is 6 words, matching the status-label wait above).
const groceryRow3 = rows3.find((r) => r.title.includes("Grocery run"));
if (!groceryRow3) throw new Error(`Grocery run row not found in note-list (rows=${rows3.map((r) => r.title).join(", ")})`);
if (groceryRow3.badge !== "6") throw new Error(`note-list badge for Grocery run=${groceryRow3.badge}, want "6"`);
console.log(`ND_BADGE_OK Grocery run row badge=${groceryRow3.badge} matches 6-word body`);

// 3. Create a fourth note distinguishable by title, to test search + pin ordering.
await app.click("new-note-button");
await app.waitForText("4 of 4 notes", { timeoutMs: 3000 });
const titleInput4 = await app.mustFind("title-input");
if (titleInput4.text !== "Untitled note") throw new Error(`second new note title wrong: ${titleInput4.text}`);
await app.setValue("title-input", "Trip itinerary");
await app.waitForText("Trip itinerary", { timeoutMs: 3000 });

const countLabel5 = await app.mustFind("note-count-label");
if (countLabel5.text !== "4 of 4 notes") throw new Error(`note-count-label=${countLabel5.text}, want "4 of 4 notes"`);

// 4. Search filter narrows the visible note-list rows.
const setSearchRes = await app.setValue("search-input", "grocery");
if (!setSearchRes.applied) throw new Error("setValue on search-input did not apply");

await app.waitForText("1 of 4 notes", { timeoutMs: 3000 });
const list6 = await app.mustFind("note-list");
const rows6 = list6.rows ?? [];
if ((list6.itemCount ?? -1) !== 1) throw new Error(`expected note-list itemCount=1 after search filter, got ${list6.itemCount}`);
if (rows6.length !== 1) throw new Error(`expected 1 note-list row after search filter, got ${rows6.length}`);
if (!rows6[0]!.title.includes("Grocery run")) throw new Error(`filtered row title=${rows6[0]!.title}, want to include "Grocery run"`);

// Clear the search filter back to showing all notes.
await app.setValue("search-input", "");
await app.waitForText("4 of 4 notes", { timeoutMs: 3000 });

// 5. Re-select "Grocery run" via the SourceList (selection is per-note
// state, independent of the search filter), then pin it via the editor
// header's pin-button (togglePinSelected() — the same path as the Note>Pin
// menu item) and assert it reorders first.
await selectNoteRow("Grocery run");
await app.waitForValue("title-input", "Grocery run", { timeoutMs: 3000 });
await app.click("pin-button");

// The click RPC ack only confirms the GTK/AppKit-side action was dispatched,
// which is synchronous at the signal level — the resulting onClick -> React
// state update -> NDP commitBatch round trip through the Bun child is a
// separate, later, async event. Give that at least one round trip before
// polling.
await new Promise((r) => setTimeout(r, 300));

// Pin gate: note-list's `rows` array is order-authoritative, so membership +
// reorder + the pinned row's star icon are all one poll — the pinned note
// ("Grocery run") must be rows[0], carrying the leading star icon main.tsx
// sets for `n.pinned`.
const rowsAfterPin = await waitForRowsOrder(
  (rows) => rows.length === 4 && (rows[0]?.title.includes("Grocery run") ?? false) && rows[0]?.iconName === "starred-symbolic",
);
console.log(`ND_PIN_ORDER_OK pinned row topmost: ${rowsAfterPin.map((r) => `${r.title}${r.iconName ? "*" : ""}`).join(" | ")}`);

// 5c. Unpin the same note (pin-button again — it's a toggle) and assert the
// full row-title order matches sortNotes's expected order once "Welcome to
// ND Notes" is the only pinned note again: pinned-first, then id descending
// among the rest.
await selectNoteRow("Grocery run");
await app.waitForValue("title-input", "Grocery run", { timeoutMs: 3000 });
await app.click("pin-button");
await new Promise((r) => setTimeout(r, 300));

const unpinnedOrder = ["Welcome to ND Notes", "Trip itinerary", "Grocery run", "Shopping list"];
const rowsAfterUnpin = await waitForRowsOrder(
  (rows) => rows.length === 4 && rows.map((r) => r.title).join(" | ") === unpinnedOrder.join(" | "),
);
console.log(`ND_UNPIN_ORDER_OK order restored: ${rowsAfterUnpin.map((r) => r.title).join(" | ")}`);

// 6. Delete the selected ("Grocery run") note; soft delete (moves to Trash),
// which drops it out of the All Notes scope the list is currently showing.
const deleteRes = await app.click("delete-note-button");
if (!deleteRes.dispatched) throw new Error("delete-note-button click did not dispatch");

await app.waitForText("3 of 3 notes", { timeoutMs: 3000 });
const list9 = await app.mustFind("note-list");
const rows9 = list9.rows ?? [];
if ((list9.itemCount ?? -1) !== 3) throw new Error(`expected note-list itemCount=3 after delete, got ${list9.itemCount}`);
if (rows9.length !== 3) throw new Error(`expected 3 note-list rows after delete, got ${rows9.length}`);
if (rows9.some((r) => r.title.includes("Grocery run"))) throw new Error("deleted (trashed) note still present in the All Notes list");

// 7. File > New Note from the menu bar must hit the SAME createNote() path
// as the editor header's new-note-button — semanticClick the menuitem and
// confirm the note-list's itemCount increments exactly like button 1 did.
const countBefore10 = (await app.mustFind("note-list")).itemCount ?? 0;
const menuClickRes = await app.click("menu-new-note");
if (!menuClickRes.dispatched) throw new Error("menu-new-note click did not dispatch");
await app.waitForText("Untitled note", { timeoutMs: 3000 });
const countAfter11 = (await app.mustFind("note-list")).itemCount ?? 0;
if (countAfter11 !== countBefore10 + 1) {
  throw new Error(`expected note-list itemCount=${countBefore10 + 1} after File>New Note, got ${countAfter11}`);
}
const titleInput11 = await app.mustFind("title-input");
if (titleInput11.text !== "Untitled note") throw new Error(`menu New Note did not select the new note: title-input=${titleInput11.text}`);
console.log(`ND_MENU_NEWNOTE_OK note-list itemCount ${countBefore10} -> ${countAfter11} via File>New Note menu item`);

// Final screenshot: screenshot()'s built-in retry (a post-mutation frame can
// race invalidation) subsumes the hand-rolled 20x/150ms poll this used to be.
const finalShot = await app.screenshot(`${shotDir}/notes-final.png`, { retries: 20 });

console.log(
  `ND_NOTES_OK create+edit+search+pin+delete round-trip verified baseline=${shot.path} final=${finalShot.path} ${finalShot.width}x${finalShot.height}`,
);
await app.close();
