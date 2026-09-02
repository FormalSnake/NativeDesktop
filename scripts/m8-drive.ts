#!/usr/bin/env bun
// scripts/m8-drive.ts: drives the M8 HMR + crash-overlay demo over the automation socket.
//
// Three modes:
//   --hmr-check:  coordinate-space check -> click increment-button x2 -> waitFor "Clicks: 2" ->
//                 print the label text -> M8_HMR_PRECHECK_OK (run BEFORE the fixture edit)
//   --hmr-verify: assert clicks-label still reads "Clicks: 2" (state preserved)
//                 AND the button's own text reflects the edited label -> M8_HMR_OK
//                 (run AFTER the fixture edit)
//   default (crash leg): assert nd-overlay-error present with non-empty text ->
//                 click nd-overlay-restart -> waitFor "Clicks:" (app re-mounted) -> M8_CRASH_OK
import { connectApp, expect } from "../packages/test/src/index.ts";

const mode = process.argv.includes("--hmr-check")
  ? "hmr-check"
  : process.argv.includes("--hmr-verify")
    ? "hmr-verify"
    : "crash";

const app = await connectApp();

if (mode === "hmr-check") {
  const tree = await app.tree();
  if (tree.coordinateSpace !== "logical-window-topleft") throw new Error("bad coordinate space");

  const btn = app.getByTestId("increment-button");
  await btn.click();
  await btn.click();

  await app.waitForText("Clicks: 2", { timeoutMs: 3000 });

  const label = app.getByTestId("clicks-label");
  console.log(`M8_HMR_PRECHECK_OK label=${JSON.stringify(await label.textContent())}`);
  await app.close();
  process.exit(0);
}

if (mode === "hmr-verify") {
  const label = app.getByTestId("clicks-label");
  const btn = app.getByTestId("increment-button");
  await expect(label).toHaveText("Clicks: 2");
  await expect(btn).toContainText("!");
  console.log(`M8_HMR_OK label=${JSON.stringify(await label.textContent())} button=${JSON.stringify(await btn.textContent())}`);
  await app.close();
  process.exit(0);
}

// crash leg (default)
const errNode = app.getByTestId("nd-overlay-error");
await expect(errNode).toBeAttached();
await expect(errNode).not.toHaveText("");
const errText = await errNode.textContent();

await app.getByTestId("nd-overlay-restart").click();
await app.waitForText("Clicks:", { timeoutMs: 5000 });

console.log(`M8_CRASH_OK error=${JSON.stringify(errText)}`);
await app.close();
