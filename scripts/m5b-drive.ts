#!/usr/bin/env bun
// scripts/m5b-drive.ts drives the gallery over the automation socket:
// setValue/type/scroll semantic actions -> React state round-trips -> waitFor.
import { connectApp, expect, poll } from "@nativedesktop/test";

const outPng = process.env.ND_SHOT_PATH ?? "/tmp/m5b-shot.png";
const app = await connectApp();

const tree = await app.tree();
if (tree.coordinateSpace !== "logical-window-topleft") throw new Error("bad coordinate space");

// 1. TextInput: setValue -> changed event -> React state -> bound label.
await app.getByTestId("name-input").fill("hello");
await app.waitForText("Echo: hello", { timeoutMs: 3000 });

// 2. type appends through GtkEditable (semantic, not keysyms).
await app.getByTestId("name-input").type(" world");
await expect(app.getByTestId("name-input")).toHaveValue("hello world");
await app.waitForText("Echo: hello world", { timeoutMs: 3000 });

// 3. TextArea via TextBuffer.
await app.getByTestId("notes-area").fill("some notes");
await app.waitForText("Notes: some notes", { timeoutMs: 3000 });

// 4. Checkbox toggled.
await app.getByTestId("agree-check").check();
await app.waitForText("Agreed: yes", { timeoutMs: 3000 });

// 5. Radio group: activating "large" must flow through toggled -> state.
await app.getByTestId("size-large").check();
await app.waitForText("Size: large", { timeoutMs: 3000 });

// 6. Slider (GtkAdjustment) -> valueChanged -> label + progressbar fraction.
await app.getByTestId("volume-slider").fill(75);
await app.waitForText("Volume: 75", { timeoutMs: 3000 });

// 7. Select (notify::selected) -> selectionChanged.
await app.getByTestId("fruit-select").selectOption(2);
await app.waitForText("Fruit: cherry", { timeoutMs: 3000 });

// 8. Scroll the ScrollView via its vertical adjustment.
const logScrollRef = await app.getByTestId("log-scroll").ref();
const scrolled = (await app.rpc.call("scroll", { ref: logScrollRef, dy: 200 })) as { x: number; y: number };
if (!(scrolled.y > 0)) throw new Error(`scroll did not move vadjustment (y=${scrolled.y})`);

// 9. Structure: grid children present even in a background tab; WebView stub in tree.
const gridNode = await app.getByTestId("layout-grid").node();
if (gridNode.children.length !== 3) throw new Error(`grid has ${gridNode.children.length} children, want 3`);
const webStubNode = await app.getByTestId("web-stub").node();
if (webStubNode.type !== "WebView") throw new Error("web-stub node is not a WebView");

// 10. Error contract: setValue on a Label must be -32602, not a crash.
let rejected = false;
try {
  await app.getByTestId("echo-label").fill("nope");
} catch {
  rejected = true; // Locator.fill surfaces JSON-RPC errors as throws (m4 pattern)
}
if (!rejected) throw new Error("setValue on a Label should be rejected");

// Screenshot polls like waitFor: right after the scroll the window's rendered
// frame is invalidated, and GtkWidgetPaintable reports "empty snapshot"
// (-32603) until the next frame lands (verified: attempt 0 fails, +200ms OK).
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

console.log(`M5B_DRIVE_OK gallery driven png=${shot.path} ${shot.width}x${shot.height}`);
app.close();
