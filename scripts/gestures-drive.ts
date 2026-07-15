#!/usr/bin/env bun
// scripts/gestures-drive.ts — drives examples/gestures over the automation
// socket and asserts the M16 surface: accessibility tree fields
// (role/enabled/focused/value), coordinate pointer input, slider drag
// through a real mouse-tracking loop, table row double-click, right-click
// with auto-dismiss, hover, keyboard typing + chords, and getTree's window
// param. macOS-only legs (input synthesis is -32003 on GTK); run via
// scripts/mac/mac-gestures.sh.
import { AutomationClient } from "../packages/mcp/src/socket.ts";
import type { JsonNode, GetTreeResult } from "../packages/react/src/generated/rpc.ts";

function find(node: JsonNode, testID: string): JsonNode | null {
  if (node.testID === testID) return node;
  for (const child of node.children) {
    const found = find(child, testID);
    if (found) return found;
  }
  return null;
}

function mustFind(tree: GetTreeResult, testID: string): JsonNode {
  const node = find(tree.root, testID);
  if (!node) throw new Error(`${testID} not found in tree`);
  return node;
}

async function waitForText(needle: string, timeoutMs = 3000): Promise<void> {
  await client.call("waitFor", { condition: { textContains: needle }, timeoutMs });
}

// Polls getTree until `pred` holds on the found node — for conditions
// waitFor's textContains can't express (numeric thresholds, a11y fields).
async function pollNode(testID: string, pred: (n: JsonNode) => boolean, what: string): Promise<JsonNode> {
  for (let i = 0; i < 30; i++) {
    const snap = (await client.call("getTree")) as GetTreeResult;
    const node = find(snap.root, testID);
    if (node && pred(node)) return node;
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`timed out waiting for ${what}`);
}

const client = await AutomationClient.connect();

// ---- leg 1: accessibility tree fields -------------------------------------
let t = (await client.call("getTree")) as GetTreeResult;
if (t.coordinateSpace !== "logical-window-topleft") throw new Error("bad coordinate space");

const rootRef = t.root.ref;
if (t.root.role !== "window") throw new Error(`root role=${t.root.role}, want window`);
if (!t.root.visible) throw new Error("root window reports invisible");

const slider = mustFind(t, "volume-slider");
if (slider.role !== "slider") throw new Error(`slider role=${slider.role}`);
if (!slider.enabled) throw new Error("slider reports disabled");
if (typeof slider.value !== "number" || Math.round(slider.value) !== 20) {
  throw new Error(`slider a11y value=${slider.value}, want 20`);
}
const check = mustFind(t, "agree-check");
if (check.value !== false) throw new Error(`checkbox a11y value=${check.value}, want false`);
const input = mustFind(t, "name-input");
if (input.focused) throw new Error("name-input focused before any interaction");
if (input.value !== "") throw new Error(`name-input a11y value=${JSON.stringify(input.value)}, want ""`);
console.log("ND_A11Y_OK role/enabled/focused/value present and correct");

// ---- leg 2: coordinate pointer (down+up on the checkbox glyph) --------------
// The checkbox frame is stretched to the stack's full width but AppKit only
// toggles on clicks over the leading glyph+label region — aim there, like a
// real user would (documented in automation-socket.md).
const cg = check.geometry!;
const cx = cg.x + Math.min(12, cg.w / 2);
const cy = cg.y + cg.h / 2;
await client.call("pointer", { phase: "down", x: cx, y: cy });
await client.call("pointer", { phase: "up", x: cx, y: cy });
await waitForText("Agreed: yes");
t = (await client.call("getTree")) as GetTreeResult;
if (mustFind(t, "agree-check").value !== true) throw new Error("checkbox a11y value did not flip after pointer click");
console.log(`ND_POINTER_OK coordinate click at (${cx},${cy}) toggled checkbox, a11y value tracked`);

// ---- leg 3: slider drag through the real NSSlider tracking loop ------------
const sg = mustFind(t, "volume-slider").geometry!;
// Thumb sits at 20% of the track; grab there-ish and pull to the right edge.
await client.call("drag", {
  fromX: sg.x + sg.w * 0.2,
  fromY: sg.y + sg.h / 2,
  toX: sg.x + sg.w - 1,
  toY: sg.y + sg.h / 2,
  steps: 16,
});
const dragged = await pollNode(
  "volume-slider",
  (n) => typeof n.value === "number" && n.value >= 80,
  "slider a11y value >= 80 after drag",
);
console.log(`ND_DRAG_OK slider dragged 20 -> ${dragged.value} via posted NSEvent sequence`);

// ---- leg 4: pointer double-click (clickCount 2) activates a table row -------
t = (await client.call("getTree")) as GetTreeResult;
const table = mustFind(t, "people-table");
const tg = table.geometry!;
// Header is ~24-28pt; row 0 sits just under it.
const rowX = tg.x + tg.w / 2;
const rowY = tg.y + 40;
await client.call("pointer", { phase: "down", x: rowX, y: rowY });
await client.call("pointer", { phase: "up", x: rowX, y: rowY });
await client.call("pointer", { phase: "down", x: rowX, y: rowY, clickCount: 2 });
await client.call("pointer", { phase: "up", x: rowX, y: rowY, clickCount: 2 });
const activated = await pollNode(
  "activated-label",
  (n) => /Activated: \d/.test(n.text ?? ""),
  "table row activation via double-click",
);
console.log(`ND_DOUBLECLICK_OK ${activated.text}`);

// ---- leg 5: doubleClick/rightClick/hover by ref (dispatch plumbing) ---------
const hoverTarget = mustFind(t, "hover-target");
const dbl = (await client.call("doubleClick", { ref: hoverTarget.ref })) as { dispatched: boolean };
if (!dbl.dispatched) throw new Error("doubleClick did not dispatch");
const rc = (await client.call("rightClick", { ref: hoverTarget.ref })) as { dispatched: boolean };
if (!rc.dispatched) throw new Error("rightClick did not dispatch");
const hv = (await client.call("hover", { ref: hoverTarget.ref })) as { dispatched: boolean };
if (!hv.dispatched) throw new Error("hover did not dispatch");
console.log("ND_RIGHTCLICK_HOVER_OK ref-targeted doubleClick/rightClick/hover dispatched");

// ---- leg 6: keyboard — click to focus, type, chord select-all + replace -----
const inputNode = mustFind(t, "name-input");
const ig = inputNode.geometry!;
await client.call("pointer", { phase: "down", x: ig.x + ig.w / 2, y: ig.y + ig.h / 2 });
await client.call("pointer", { phase: "up", x: ig.x + ig.w / 2, y: ig.y + ig.h / 2 });
await client.call("keys", { keys: "Hi" });
await waitForText("Echo: Hi");
t = (await client.call("getTree")) as GetTreeResult;
if (!mustFind(t, "name-input").focused) throw new Error("name-input not focused after pointer click");
await client.call("keys", { keys: "cmd+a" });
await client.call("keys", { keys: "x" });
await waitForText("Echo: x");
console.log("ND_KEYS_OK typed 'Hi', cmd+a chord + 'x' replaced selection");

// ---- leg 7: getTree window param -------------------------------------------
const scoped = (await client.call("getTree", { window: rootRef })) as GetTreeResult;
if (scoped.root.ref !== rootRef) throw new Error("window-scoped getTree returned wrong root");
let badRefRejected = false;
try {
  await client.call("getTree", { window: 999999 });
} catch {
  badRefRejected = true;
}
if (!badRefRejected) throw new Error("getTree accepted an unknown window ref");
console.log("ND_WINDOWPARAM_OK scoped snapshot + unknown-ref rejection");

console.log("ND_GESTURES_OK a11y tree + pointer + drag + doubleClick + rightClick + hover + keys verified");
client.close();
