#!/usr/bin/env bun
// scripts/mac/cursor-drive.ts: app.cursor drives the real system cursor
// (packages/test/src/cursor.ts over the host's `--nd-input` helper). Each leg
// acts only through the cursor and checks the app's own state, so a click that
// lands a few points off fails here: a checkbox, a slider dragged through
// AppKit's tracking loop, a table row double-clicked, and a text field's
// native context menu, which the NSEvent-posting rightClick RPC cannot open,
// confirmed by a region capture compositing the menu window. Needs the host
// binary granted (`NDShell --nd-grant` with SIP off, or System Settings).
// Moves the user's cursor. Marker: ND_CURSOR_OK.
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { expect, launchApp } from "../../packages/test/src/index.ts";

const out = process.env.ND_CURSOR_OUT ?? "/tmp/nd-cursor-drive";
mkdirSync(out, { recursive: true });
const hostBinary = process.env.ND_HOST_BINARY ?? resolve(import.meta.dir, "../../swift/.build/release/NDShell");

// ---- leg 1: examples/gestures, click + drag + type + double-click ----------
{
  const app = await launchApp({ entry: "examples/gestures/main.tsx", hostBinary, logPath: `${out}/gestures.log` });
  try {
    await expect(app.getByTestId("agree-label")).toHaveText("Agreed: no");
    await app.cursor.click(app.getByTestId("agree-check"));
    await expect(app.getByTestId("agree-label")).toHaveText("Agreed: yes");
    console.log("ND_CURSOR_CLICK_OK checkbox toggled by a real click");

    const slider = app.getByTestId("volume-slider");
    const box = (await slider.boundingBox())!;
    const before = await app.getByTestId("volume-label").textContent();
    await app.cursor.drag(
      { x: box.x + box.width * 0.5, y: box.y + box.height / 2 },
      { x: box.x + box.width * 0.9, y: box.y + box.height / 2 },
    );
    const after = await app.getByTestId("volume-label").textContent();
    if (before === after) throw new Error(`slider drag left the value at ${after}`);
    console.log(`ND_CURSOR_DRAG_OK ${before} -> ${after}`);

    await app.cursor.click(app.getByTestId("name-input"));
    await app.keyboard.type("ok");
    await expect(app.getByTestId("echo-label")).toHaveText("Echo: ok");
    console.log("ND_CURSOR_FOCUS_OK a real click focused the field");

    const table = (await app.getByTestId("people-table").boundingBox())!;
    // First body row: below the header, left of centre.
    await app.cursor.dblclick({ x: table.x + 40, y: table.y + 38 });
    await expect(app.getByTestId("activated-label")).not.toHaveText("Activated: -1");
    console.log(`ND_CURSOR_DBLCLICK_OK ${await app.getByTestId("activated-label").textContent()}`);
  } finally {
    await app.close();
  }
}

// ---- leg 2: examples/locators, a native context menu -----------------------
{
  let composited = 0;
  const app = await launchApp({
    entry: "examples/locators/main.tsx",
    hostBinary,
    env: { ND_AUTOMATION_CAPTURE: "region" },
    logPath: `${out}/locators.log`,
    onStderr: (line) => {
      const m = /ND_SNAPSHOT_REGION windows=(\d+)/.exec(line);
      if (m) composited = Number(m[1]);
    },
  });
  try {
    const input = app.getByTestId("query-input");
    await app.cursor.rightClick(input);
    await Bun.sleep(500);
    await app.screenshot(`${out}/context-menu.png`);
    await Bun.sleep(100);
    if (composited < 2) throw new Error(`no menu window in the capture (${composited} composited)`);
    console.log(`ND_CURSOR_CONTEXT_MENU_OK ${composited} windows composited, ${out}/context-menu.png`);
    // Dismiss by clicking elsewhere in the window, as a user would.
    await app.cursor.click(app.getByTestId("folder-label"));
  } finally {
    await app.close();
  }
}

console.log("ND_CURSOR_OK");
