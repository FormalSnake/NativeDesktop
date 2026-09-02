#!/usr/bin/env bun
// scripts/locator-drive.ts: drives examples/locators over the automation
// socket, exercising the Playwright-shaped surface through locators only, so
// the host RPCs behind it (focus, scrollIntoView, snapshotNode,
// setWindowFrame) and the a11y fields the matchers read (checked, options,
// placeholder) are covered end to end rather than in unit tests alone.
// Run via scripts/mac/mac-gestures.sh on macOS and scripts/headless-locators.sh
// on Linux; both own the host and hand over ND_AUTOMATION_SOCKET. Every leg is
// backend-neutral except the chord half of leg 1: press() and keyboard.type()
// ride the `keys` RPC, which GTK answers -32003, so ND_BACKEND=gtk skips it.
import { connectApp, expect, poll } from "../packages/test/src/index.ts";

const gtk = process.env.ND_BACKEND === "gtk";
const app = await connectApp();

// ---- leg 1: focus() + toBeFocused, then press() a chord --------------------
const input = app.getByTestId("query-input");
await expect(input).not.toBeFocused();
await input.focus();
await expect(input).toBeFocused();

await input.fill("hello");
await expect(app.getByTestId("query-label")).toHaveText("Query: hello");
if (gtk) {
  console.log("SKIP press('Meta+a') + keyboard.type: the keys RPC answers -32003 on GTK");
  console.log("ND_LOCATOR_FOCUS_OK focus() + toBeFocused, fill() through the label");
} else {
  // press() focuses first, then sends the chord: cmd+a selects the field, so
  // the next character replaces the whole value instead of appending.
  await input.press("Meta+a");
  await app.keyboard.type("z");
  await expect(app.getByTestId("query-label")).toHaveText("Query: z");
  console.log("ND_LOCATOR_FOCUS_OK focus() + toBeFocused, press('Meta+a') replaced the selection");
}

// ---- leg 2: scrollIntoViewIfNeeded on a clipped row ------------------------
const lastRow = app.getByTestId("row-39");
await expect(lastRow).toBeAttached();
if (await lastRow.isVisible()) throw new Error("row-39 was already inside the clip; nothing to scroll");
await lastRow.scrollIntoViewIfNeeded();
await expect(lastRow).toBeVisible();
console.log("ND_LOCATOR_SCROLL_OK row-39 scrolled from clipped to visible");

// ---- leg 3: locator.screenshot() is node-sized, not window-sized -----------
const shot = await lastRow.screenshot();
const box = (await lastRow.boundingBox())!;
if (shot.width <= 0 || shot.height <= 0) throw new Error(`snapshotNode gave ${shot.width}x${shot.height}`);
// The PNG is in device pixels, the box in logical points: the ratio is the
// window's backing scale, so compare through it rather than for equality.
const scale = shot.width / box.width;
if (scale < 0.9 || scale > 4.1) throw new Error(`snapshotNode scale ${scale} off (${shot.width} vs ${box.width})`);
if (Math.abs(shot.height / box.height - scale) > 0.25) {
  throw new Error(`snapshotNode aspect off: ${shot.width}x${shot.height} vs ${box.width}x${box.height}`);
}
const rootBefore = (await app.getByRole("window").boundingBox())!;
if (shot.width >= rootBefore.width * scale) throw new Error("snapshotNode captured the window, not the node");
console.log(`ND_LOCATOR_SNAPSHOT_OK ${shot.width}x${shot.height} px for a ${box.width}x${box.height} pt row`);

// ---- leg 4: setWindowSize, and the root box follows ------------------------
const info = await app.setWindowSize(900, 600);
if (info.geometry === null) throw new Error("setWindowFrame answered no geometry");
if (info.geometry.w !== 900 || info.geometry.h !== 600) {
  throw new Error(`setWindowFrame answered ${info.geometry.w}x${info.geometry.h}`);
}
const root = app.getByRole("window");
const rootAfter = await poll(
  async () => (await root.boundingBox())!,
  (b) => b.width === 900 && b.height === 600,
);
console.log(`ND_LOCATOR_FRAME_OK window ${rootBefore.width}x${rootBefore.height} -> ${rootAfter.width}x${rootAfter.height}`);

// ---- leg 5: isChecked across check()/uncheck() -----------------------------
const notify = app.getByTestId("notify-check");
if (await notify.isChecked()) throw new Error("notify-check started checked");
await notify.check();
await expect(notify).toBeChecked();
await expect(app.getByTestId("notify-label")).toHaveText("Notify: on");
if (!(await notify.isChecked())) throw new Error("isChecked() disagreed with toBeChecked()");
await notify.uncheck();
await expect(notify).not.toBeChecked();
if (await notify.isChecked()) throw new Error("uncheck() left isChecked() true");
console.log("ND_LOCATOR_CHECK_OK isChecked tracked check() and uncheck()");

// ---- leg 6: selectOption by its label, not its index -----------------------
const folder = app.getByTestId("folder-select");
await expect(app.getByTestId("folder-label")).toHaveText("Folder: Home");
await folder.selectOption("Downloads");
await expect(app.getByTestId("folder-label")).toHaveText("Folder: Downloads");
await expect(folder).toHaveValue("2");
console.log("ND_LOCATOR_SELECT_OK selectOption('Downloads') resolved through the node's options");

console.log("ND_LOCATOR_OK focus + press + scrollIntoView + snapshotNode + setWindowFrame + check + selectOption verified");
await app.close();
