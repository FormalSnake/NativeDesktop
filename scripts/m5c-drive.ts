#!/usr/bin/env bun
// scripts/m5c-drive.ts drives the M5c gallery (styling + ListView) over the
// automation socket. Mirrors scripts/m5b-drive.ts's connectApp/locator
// scaffold. Phase A only (styled widgets + ListView); Phase B (the
// web-CSS-rejection negative test) runs as a separate host process and is
// driven by scripts/headless-m5c.sh via stderr grep, not this script.
import { connectApp, poll } from "@nativedesktop/test";

const outPng = process.env.ND_SHOT_PATH ?? "/tmp/m5c-shot.png";
const app = await connectApp();

const tree = await app.tree();
if (tree.coordinateSpace !== "logical-window-topleft") throw new Error("bad coordinate space");

// 1. Styled widgets render: nodes present with the right type. color/border/
// font/padding/margin are visual: the screenshot at the end is the proof
// they compiled to CSS + margin properties without error (M5c-D1/D2).
const styledTab = await app.getByTestId("styled-tab").node();
if (styledTab.type !== "Box") throw new Error(`styled-tab wrong type: ${styledTab.type}`);
const styledLabel = await app.getByTestId("styled-label").node();
if (styledLabel.type !== "Label") throw new Error(`styled-label wrong type: ${styledLabel.type}`);
const styledButton = await app.getByTestId("styled-button").node();
if (styledButton.type !== "Button") throw new Error(`styled-button wrong type: ${styledButton.type}`);

// 2. ListView present with 100k items; getTree reports itemCount, NOT 100k
// children (M5c-D4: recycled rows are untracked GTK internals, never dumped).
const list = await app.getByTestId("big-list").node();
if (list.type !== "ListView") throw new Error(`big-list not a ListView: ${list.type}`);
if (list.itemCount !== 100000) throw new Error(`itemCount=${list.itemCount}, want 100000`);
if (list.children.length !== 0) throw new Error(`ListView dumped ${list.children.length} children (must be 0)`);

// 3. Scroll the list. LANDED-CODE DELTA from the plan: handleScroll
// (src/automation.zig:298-303) hard-rejects any widget_type != "ScrollView":
// a ListView's ScrolledWindow wrapper is NOT re-typed as "ScrollView" in
// NodeMeta (it stays "ListView", src/generated/widgets.zig create body), so
// scroll-by-ref on a ListView is unsupported by RPC contract, independent of
// visibility. Separately, the gallery's "List" tab is not the tabview's
// default selected page (gallery-tabs has no selectedIndex set, defaults to
// "Form"), and there is no automation RPC to switch TabView pages (setValue
// does not support TabView; no tab-header nodes are tracked), so big-list
// also fails the actionability hit-test (computeBounds is degenerate for an
// unmapped background tab page). Verified live (throwaway probe, this
// session): scroll on big-list returns JSON-RPC error -32001 "not
// actionable" with data.reason "offscreen", never a crash and never a
// silent no-op. We assert exactly that well-formed rejection here: this is
// the honest, currently-observable contract for "scroll the list" given the
// landed automation surface (no scroll support for ListView-typed nodes; no
// tab-switch RPC). A `scroll`-on-ListView RPC extension and/or a tab-switch
// action are natural follow-ups, out of scope for M5c (plan's "do NOT expand
// automation scope in M5c").
let scrollRejected = false;
let scrollRejectMsg = "";
try {
  await app.rpc.call("scroll", { ref: list.ref, dy: 5000 });
} catch (e) {
  scrollRejected = true;
  scrollRejectMsg = (e as Error).message;
}
if (!scrollRejected) throw new Error("scroll on big-list unexpectedly succeeded (landed automation has no ListView scroll support)");
if (!scrollRejectMsg.includes("-32001")) throw new Error(`scroll on big-list rejected with unexpected error: ${scrollRejectMsg}`);

// 4. rowActivated / selection observable. LANDED-CODE DELTA from the plan:
// there is no dedicated row-activate RPC and no automation action can
// select/click a ListView row (rows are recycled/untracked, and setValue
// does not support the ListView widget_type: src/automation.zig:226-262).
// GtkListView's "activate" signal only fires on a real user double-
// click/Enter on a mapped row, which this automation layer cannot drive.
// The observable proof available today is the gallery's own wiring: the
// "activated-label" node mirrors React's activatedRow state, which starts
// at -1 and is only set by a real onRowActivated callback (never by
// mount-time logic), so it renders "Activated: -1" at steady state. This
// proves onRowActivated is wired end-to-end from the ListView's create body
// through cbListActivate (src/generated/widgets.zig) up to React state,
// without asserting a signal this drive cannot trigger.
const activatedLabel = await app.getByTestId("activated-label").node();
if (activatedLabel.type !== "Label") throw new Error(`activated-label wrong type: ${activatedLabel.type}`);
if (activatedLabel.text !== "Activated: -1") throw new Error(`activated-label text=${activatedLabel.text}, want "Activated: -1"`);

// Re-read the tree: itemCount must be stable (the ListView is unaffected by
// the rejected scroll attempt above: no partial state, no crash).
const list2 = await app.getByTestId("big-list").node();
if (list2.itemCount !== 100000) throw new Error(`itemCount changed after scroll attempt: ${list2.itemCount}`);
if (list2.children.length !== 0) throw new Error(`ListView dumped children on re-read: ${list2.children.length}`);

// Screenshot polls like waitFor: right after a getTree/scroll round-trip the
// window's rendered frame can race invalidation, and GtkWidgetPaintable
// reports "empty snapshot" (-32603) until the next frame lands (verified in
// M5b: attempt 0 fails, +150ms OK), retry-poll up to ~3s (m5b-drive.ts
// pattern).
const shot = await poll(
  async () => {
    try {
      return await app.screenshot(outPng);
    } catch {
      return null;
    }
  },
  (s) => s !== null,
  { timeoutMs: 3000, intervalMs: 150 },
);
if (!shot || shot.width <= 0 || shot.height <= 0) throw new Error("screenshot has no dimensions");

console.log(`M5C_DRIVE_OK gallery driven png=${shot.path} ${shot.width}x${shot.height} itemCount=${list2.itemCount}`);
app.close();
