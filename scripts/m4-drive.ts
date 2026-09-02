#!/usr/bin/env bun
// scripts/m4-drive.ts connects ND_AUTOMATION_SOCKET, drives the counter, asserts, screenshots.
//
// Two modes:
//   default: getByTestId("increment-button") -> click x3 -> waitFor "Clicks: 3" -> screenshot -> assert
//   --slo:   getTree + screenshot only (used while the JS child is SIGSTOPped for the D11 SLO test)
import { connectApp } from "@nativedesktop/test";

const slo = process.argv.includes("--slo");
const outPng = process.env.ND_SHOT_PATH ?? "/tmp/m4-shot.png";
const app = await connectApp();

if (slo) {
  const tree = await app.tree();
  if (!tree || tree.coordinateSpace !== "logical-window-topleft") {
    throw new Error("bad getTree result under stall");
  }
  const shot = await app.screenshot(outPng);
  if (shot.width <= 0 || shot.height <= 0) throw new Error("screenshot has no dimensions under stall");
  console.log(`M4_SLO_OK tree+screenshot answered while child stalled png=${shot.path} ${shot.width}x${shot.height}`);
  app.close();
  process.exit(0);
}

const tree = await app.tree();
if (tree.coordinateSpace !== "logical-window-topleft") throw new Error("bad coordinate space");

const incrementButton = app.getByTestId("increment-button");
for (let i = 0; i < 3; i++) {
  await incrementButton.click();
}

await app.waitForText("Clicks: 3", { timeoutMs: 3000 });

const shot = await app.screenshot(outPng);
if (shot.width <= 0 || shot.height <= 0) throw new Error("screenshot has no dimensions");

console.log(`M4_DRIVE_OK clicks=3 png=${shot.path} ${shot.width}x${shot.height}`);
app.close();
