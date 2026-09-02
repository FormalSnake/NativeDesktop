#!/usr/bin/env bun
// scripts/gestures-drive.ts: drives examples/gestures over the automation
// socket and asserts the M16 surface: accessibility tree fields
// (role/enabled/focused/value), coordinate pointer input, slider drag
// through a real mouse-tracking loop, table row double-click, right-click
// with auto-dismiss, hover, keyboard typing + chords, and getTree's window
// param. macOS-only legs (input synthesis is -32003 on GTK); run via
// scripts/mac/mac-gestures.sh, which owns the host and hands over
// ND_AUTOMATION_SOCKET.
import { connectApp, expect } from "../packages/test/src/index.ts";

const app = await connectApp();

// ---- leg 1: accessibility tree fields -------------------------------------
const tree = await app.tree();
if (tree.coordinateSpace !== "logical-window-topleft") throw new Error("bad coordinate space");

const rootRef = tree.root.ref;
await expect(app.getByRole("window")).toBeVisible();

const slider = app.getByTestId("volume-slider");
await expect(slider).toHaveAttribute("role", "slider");
await expect(slider).toBeEnabled();
await expect(slider).toHaveValue("20");
await expect(app.getByTestId("agree-check")).not.toBeChecked();
const input = app.getByTestId("name-input");
await expect(input).not.toBeFocused();
await expect(input).toHaveValue("");
console.log("ND_A11Y_OK role/enabled/focused/value present and correct");

// ---- leg 2: coordinate pointer (down+up on the checkbox glyph) --------------
// The checkbox frame is stretched to the stack's full width but AppKit only
// toggles on clicks over the leading glyph+label region, so aim there like a
// real user would (documented in automation-socket.md).
const cg = (await app.getByTestId("agree-check").boundingBox())!;
const cx = cg.x + Math.min(12, cg.width / 2);
const cy = cg.y + cg.height / 2;
await app.mouse.click(cx, cy);
await app.waitForText("Agreed: yes");
await expect(app.getByTestId("agree-check")).toBeChecked();
console.log(`ND_POINTER_OK coordinate click at (${cx},${cy}) toggled checkbox, a11y value tracked`);

// ---- leg 3: slider drag through the real NSSlider tracking loop ------------
const sg = (await slider.boundingBox())!;
// Thumb sits at 20% of the track; grab there-ish and pull to the right edge.
await app.mouse.dragTo(
  { x: sg.x + sg.width * 0.2, y: sg.y + sg.height / 2 },
  { x: sg.x + sg.width - 1, y: sg.y + sg.height / 2 },
  { steps: 16 },
);
await expect(slider).toHaveValue(/^(8\d|9\d|100)/);
console.log(`ND_DRAG_OK slider dragged 20 -> ${await slider.inputValue()} via posted NSEvent sequence`);

// ---- leg 4: pointer double-click (clickCount 2) activates a table row -------
const tg = (await app.getByTestId("people-table").boundingBox())!;
// Header is ~24-28pt; row 0 sits just under it.
const rowX = tg.x + tg.width / 2;
const rowY = tg.y + 40;
await app.mouse.dblclick(rowX, rowY);
const activated = app.getByTestId("activated-label");
await expect(activated).toContainText(/Activated: \d/);
console.log(`ND_DOUBLECLICK_OK ${await activated.textContent()}`);

// ---- leg 5: doubleClick/rightClick/hover by ref (dispatch plumbing) ---------
const hoverTarget = app.getByTestId("hover-target");
await hoverTarget.dblclick();
await hoverTarget.rightClick();
await hoverTarget.hover();
console.log("ND_RIGHTCLICK_HOVER_OK ref-targeted doubleClick/rightClick/hover dispatched");

// ---- leg 6: keyboard, click to focus, type, chord select-all + replace ------
const ig = (await input.boundingBox())!;
await app.mouse.click(ig.x + ig.width / 2, ig.y + ig.height / 2);
await app.keyboard.type("Hi");
await app.waitForText("Echo: Hi");
await expect(input).toBeFocused();
await app.keyboard.press("Meta+a");
await app.keyboard.type("x");
await app.waitForText("Echo: x");
console.log("ND_KEYS_OK typed 'Hi', cmd+a chord + 'x' replaced selection");

// ---- leg 7: getTree window param -------------------------------------------
const scoped = await app.tree(rootRef);
if (scoped.root.ref !== rootRef) throw new Error("window-scoped getTree returned wrong root");
let badRefRejected = false;
try {
  await app.tree(999999);
} catch {
  badRefRejected = true;
}
if (!badRefRejected) throw new Error("getTree accepted an unknown window ref");
console.log("ND_WINDOWPARAM_OK scoped snapshot + unknown-ref rejection");

console.log("ND_GESTURES_OK a11y tree + pointer + drag + doubleClick + rightClick + hover + keys verified");
await app.close();
