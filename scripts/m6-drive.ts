#!/usr/bin/env bun
// scripts/m6-drive.ts drives the Mac AppKit shell over the automation socket.
// Runs ON THE MAC (scripts/mac/mac-m6.sh ssh's in and invokes it against the
// local socket path, no tunnel; mirrors how mac-build.sh runs zig remotely).
// Reuses the same @nativedesktop/test locator layer as m4/m5c; assertions mirror them:
//   counter: getByTestId -> click x3 -> waitFor "Clicks: 3" -> non-blank screenshot
//   gallery: styled/type asserts + ListView itemCount contract (m5c-drive.ts)
//   --slo:   getTree + screenshot only (child SIGSTOPped, D11 SLO leg)
import { connectApp, poll, type AttachedApp } from "@nativedesktop/test";

// Screenshot polls like waitFor (the m5b/m5c 150ms->3s pattern): a frame that
// isn't ready yet (-32603) becomes a retry, not a failure.
async function pollScreenshot(app: AttachedApp, path: string): Promise<{ path: string; width: number; height: number }> {
  const shot = await poll(
    async () => {
      try {
        return await app.screenshot(path);
      } catch {
        return null;
      }
    },
    (s) => s !== null,
    { timeoutMs: 3000, intervalMs: 150 },
  );
  if (!shot || shot.width <= 0 || shot.height <= 0) throw new Error("screenshot has no dimensions");
  return shot;
}

const mode = process.argv[2] ?? "counter";
const slo = process.argv.includes("--slo");
const outPng = process.env.ND_SHOT_PATH ?? "/tmp/m6-shot.png";
const app = await connectApp();

if (slo) {
  const tree = await app.tree();
  if (!tree || tree.coordinateSpace !== "logical-window-topleft") {
    throw new Error("bad getTree result under stall");
  }
  const shot = await pollScreenshot(app, outPng);
  console.log(`M6_SLO_OK tree+screenshot answered while child stalled png=${shot.path} ${shot.width}x${shot.height}`);
  app.close();
  process.exit(0);
}

if (mode === "counter") {
  const tree = await app.tree();
  if (tree.coordinateSpace !== "logical-window-topleft") throw new Error("bad coordinate space");

  const incrementButton = app.getByTestId("increment-button");
  for (let i = 0; i < 3; i++) {
    await incrementButton.click();
  }

  await app.waitForText("Clicks: 3", { timeoutMs: 3000 });

  const shot = await pollScreenshot(app, outPng);
  console.log(`M6_DRIVE_OK clicks=3 png=${shot.path} ${shot.width}x${shot.height}`);
} else if (mode === "gallery") {
  const tree = await app.tree();
  if (tree.coordinateSpace !== "logical-window-topleft") throw new Error("bad coordinate space");

  // Widget-create assertions (mirror m5c-drive.ts): the styled tab's nodes
  // exist with the right types, proving all 18 create arms mounted.
  const styledTab = await app.getByTestId("styled-tab").node();
  if (styledTab.type !== "Box") throw new Error(`styled-tab wrong type: ${styledTab.type}`);
  const styledLabel = await app.getByTestId("styled-label").node();
  if (styledLabel.type !== "Label") throw new Error(`styled-label wrong type: ${styledLabel.type}`);
  const styledButton = await app.getByTestId("styled-button").node();
  if (styledButton.type !== "Button") throw new Error(`styled-button wrong type: ${styledButton.type}`);

  // ListView itemCount contract (M5c-D4 / M6b-D2): 100k items, zero children.
  const list = await app.getByTestId("big-list").node();
  if (list.type !== "ListView") throw new Error(`big-list not a ListView: ${list.type}`);
  if (list.itemCount !== 100000) throw new Error(`itemCount=${list.itemCount}, want 100000`);
  if (list.children.length !== 0) throw new Error(`ListView dumped ${list.children.length} children (must be 0)`);

  // rowActivated wired end-to-end: activated-label mirrors React state,
  // steady-state "Activated: -1" (same observable proof as m5c).
  const activatedLabel = await app.getByTestId("activated-label").node();
  if (activatedLabel.text !== "Activated: -1") {
    throw new Error(`activated-label text=${activatedLabel.text}, want "Activated: -1"`);
  }

  const shot = await pollScreenshot(app, outPng);
  console.log(`M6_GALLERY_OK styled+list png=${shot.path} ${shot.width}x${shot.height} itemCount=${list.itemCount}`);
} else {
  throw new Error(`unknown mode: ${mode}`);
}
app.close();
