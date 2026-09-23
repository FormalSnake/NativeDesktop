#!/usr/bin/env bun
// scripts/mac/header-palette-drive.ts: examples/command-palette/header-probe.tsx,
// a browser-shaped window. Leg 1: a search field packed into the header bar
// (it lands in the NSToolbar) is reachable through automation: the locator
// resolves it as actionable, fill and inputValue round-trip, focus gives it the
// keyboard, and a real cursor click puts the caret in it so typed keys and
// Return reach the app. Leg 2: with that field holding the keyboard, the
// palette's Cmd+K accelerator presents the palette and its field takes the
// keys. Leg 3: Cmd+L on a field the user is already typing in selects all of
// it, so the next keystrokes replace the address. Region captures show each.
// Moves the user's cursor: hold the mac gate lock and grant the host once
// (`NDShell --nd-grant`). Marker: ND_HEADER_PALETTE_OK.
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { expect, launchApp } from "../../packages/test/src/index.ts";

const out = process.env.ND_HEADER_PALETTE_OUT ?? "/tmp/nd-header-palette";
mkdirSync(out, { recursive: true });
const hostBinary = process.env.ND_HOST_BINARY ?? resolve(import.meta.dir, "../../swift/.build/release/NDShell");

let composited = 0;
const app = await launchApp({
  entry: "examples/command-palette/header-probe.tsx",
  hostBinary,
  env: { ND_AUTOMATION_CAPTURE: "region" },
  logPath: `${out}/host.log`,
  onStderr: (line) => {
    const m = /ND_SNAPSHOT_REGION windows=(\d+)/.exec(line);
    if (m) composited = Number(m[1]);
  },
});

type Box = { x: number; y: number; width: number; height: number };

// The header row at one window width: every item on screen and in the window
// (none pushed into the overflow menu), in order without overlap, the field
// wider than its neighbours, and all of them on one centre line.
async function checkHeader(width: number): Promise<void> {
  await app.setWindowSize(width, 640);
  await Bun.sleep(600);
  const ids = ["layout-toggle", "reload", "security", "omnibox", "extensions", "downloads"];
  const boxes: Box[] = [];
  for (const id of ids) {
    const loc = app.getByTestId(id);
    await expect(loc).toBeVisible();
    const b = await loc.boundingBox();
    if (!b || b.width < 1 || b.x < 0 || b.x + b.width > width) {
      throw new Error(`${width}pt: ${id} is off the header row at ${JSON.stringify(b)}`);
    }
    boxes.push(b);
  }
  for (let i = 1; i < boxes.length; i++) {
    if (boxes[i]!.x < boxes[i - 1]!.x + boxes[i - 1]!.width) {
      throw new Error(`${width}pt: ${ids[i]} overlaps ${ids[i - 1]} (${JSON.stringify(boxes)})`);
    }
  }
  const field = boxes[3]!;
  if (field.width < 200) throw new Error(`${width}pt: the field is ${field.width}pt wide`);
  const mid = (b: Box) => b.y + b.height / 2;
  for (let i = 0; i < boxes.length; i++) {
    if (Math.abs(mid(boxes[i]!) - mid(field)) > 1) {
      throw new Error(`${width}pt: ${ids[i]} sits off the field's centre line (${JSON.stringify(boxes)})`);
    }
  }
  await app.screenshot(`${out}/header-${width}.png`);
  console.log(`ND_HEADER_LAYOUT_OK ${width}pt field ${field.width}pt, ${out}/header-${width}.png`);
}

try {
  const omnibox = app.getByTestId("omnibox");
  await checkHeader(1280);
  await checkHeader(820);
  await app.setWindowSize(1280, 720);
  await Bun.sleep(600);

  // ---- leg 1: the header field is automatable ------------------------------
  const box = await omnibox.boundingBox();
  if (!box || box.width < 100) throw new Error(`omnibox geometry ${JSON.stringify(box)}`);
  await omnibox.fill("example.org");
  const filled = await omnibox.inputValue();
  if (filled !== "example.org") throw new Error(`inputValue after fill is ${JSON.stringify(filled)}`);
  await omnibox.focus();
  await expect(omnibox).toBeFocused();
  console.log(`ND_HEADER_FIELD_FILL_OK fill/inputValue/focus at ${JSON.stringify(box)}`);

  await omnibox.fill("");
  await app.cursor.click(omnibox);
  await expect(omnibox).toBeFocused();
  await app.keyboard.type("nd.dev");
  await app.keyboard.press("Enter");
  await expect(app.getByTestId("committed-label")).toHaveText("Committed: nd.dev");
  await app.screenshot(`${out}/01-header-field.png`);
  console.log(`ND_HEADER_FIELD_CURSOR_OK a real click took the keyboard, ${out}/01-header-field.png`);

  // ---- leg 2: Cmd+K presents the palette over the focused header field ----
  await app.cursor.click(omnibox);
  await expect(omnibox).toBeFocused();
  await app.keyboard.press("Meta+k");
  const palette = app.getByTestId("palette");
  await expect(palette).toBeVisible();
  // The palette's field has the keyboard: typing lands in the palette, not in
  // the header field behind it.
  // The palette at a narrow width too, captured for the eye check.
  await app.setWindowSize(820, 640);
  await Bun.sleep(600);
  await expect(palette).toBeVisible();
  await app.screenshot(`${out}/02-palette-820.png`);
  await app.setWindowSize(1280, 720);
  await Bun.sleep(600);
  await app.keyboard.type("rel");
  const inHeader = await omnibox.inputValue();
  if (inHeader.includes("rel")) throw new Error(`typing went to the header field (${JSON.stringify(inHeader)})`);
  await app.screenshot(`${out}/02-palette.png`);
  if (composited < 1) throw new Error("no region capture");
  await app.keyboard.press("Enter");
  await expect(app.getByTestId("picked-label")).toHaveText("Picked: reload");
  await expect(palette).toBeHidden();
  console.log(`ND_HEADER_PALETTE_KEY_OK Cmd+K presented over the focused field, ${out}/02-palette.png`);

  // ---- leg 3: Cmd+L selects the whole address ------------------------------
  // The caret sits at the end of typed text with nothing selected, so a
  // focus that only makes the field first responder (it already is) would
  // leave the typing appended.
  await omnibox.fill("");
  await app.cursor.click(omnibox);
  await expect(omnibox).toBeFocused();
  await app.keyboard.type("old.example");
  await app.keyboard.press("Meta+l");
  // The selection is the app's answer to the menu item, a round trip later;
  // keys typed inside that window reach the field before it and append. The
  // Linux leg (focusShortcut) waits the same way.
  await Bun.sleep(800);
  await app.keyboard.type("new.dev");
  await Bun.sleep(300);
  const replaced = await omnibox.inputValue();
  if (replaced !== "new.dev") throw new Error(`Cmd+L left ${JSON.stringify(replaced)}, want "new.dev"`);
  await app.screenshot(`${out}/03-select-all.png`);
  console.log(`ND_HEADER_SELECT_ALL_OK Cmd+L selected the whole field, ${out}/03-select-all.png`);

  console.log("ND_HEADER_PALETTE_OK");
} finally {
  await app.close();
}
