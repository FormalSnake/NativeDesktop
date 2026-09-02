#!/usr/bin/env bun
// scripts/panes-drive.ts: drives examples/panes over the automation socket
// (the host and the shared ND_STORE_DIR belong to scripts/headless-panes.sh).
//
// Default mode builds the layout: split H, split V, close the third pane,
// assert the tree shape (nested paned nodes, leaves in place), set the root
// ratio through the app button and assert the model echoes it, then leave
// time for the store debounce -> ND_PANES_OK.
// --restore (second host run against the same ND_STORE_DIR) asserts the
// persisted model came back: two panes, ratio 0.30, pane 3 gone ->
// ND_PANES_RESTORE_OK.
import { connectApp, expect, poll } from "../packages/test/src/index.ts";

// 15s ceiling: software-rendered weston on a loaded CI runner lands the ratio
// echo well past 3s; an upper bound costs nothing when fast.
const T = 15_000;

const app = await connectApp();
app.actionTimeout = T;

const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));
const restore = process.argv.includes("--restore");

if (!restore) {
  await app.waitForText("pane:1", { timeoutMs: T });

  await app.getByTestId("split-h-1").click(); // s2[1, 2]
  await app.waitForText("pane:2", { timeoutMs: T });
  await app.getByTestId("split-v-2").click(); // s2[1, s3[2, 3]]
  await app.waitForText("pane:3", { timeoutMs: T });

  await expect(app.locator("testid=panes-split-s2 >> testid=panes-split-s3")).toHaveCount(1);
  await expect(app.locator("testid=panes-split-s3 >> testid=panes-leaf-2")).toHaveCount(1);
  await expect(app.locator("testid=panes-split-s3 >> testid=panes-leaf-3")).toHaveCount(1);
  await expect(app.getByTestId("panes-leaf-1")).toBeVisible();

  // Nested-layout regression gate (AppKit once collapsed BOTH outer panes to
  // width 0 when the inner split attached): every leaf must settle at real
  // geometry. Polled, because the divider re-apply after a structural change
  // is deferred a runloop turn on AppKit and the first read can race it.
  const leafIDs = ["panes-leaf-1", "panes-leaf-2", "panes-leaf-3"];
  const boxes = await poll(
    () => Promise.all(leafIDs.map((id) => app.getByTestId(id).boundingBox())),
    (found) => found.every((b) => b != null && b.width >= 50 && b.height >= 50),
    { timeoutMs: T, intervalMs: 150 },
  ).catch(async () => {
    const report = await Promise.all(leafIDs.map(async (id) => `${id}=${JSON.stringify(await app.getByTestId(id).boundingBox())}`));
    throw new Error(`nested split leaves never got real geometry: ${report.join(" ")}`);
  });
  void boxes;

  await app.getByTestId("close-3").click(); // back to s2[1, 2], focus moves to 2
  await app.waitForText("panes:2", { timeoutMs: T });
  await expect(app.getByTestId("pane-label-3")).toHaveCount(0);
  await expect(app.getByTestId("panes-split-s3")).toHaveCount(0);
  await expect(app.getByTestId("pane-label-2")).toContainText("(focused)");

  await app.getByTestId("ratio-30").click();
  await app.waitForText("ratio:0.30", { timeoutMs: T });

  // Native positionChanged echo sanity: the left pane's box should sit near
  // 30% of the split's width (wide bounds; theming/handle width varies).
  const leaf1 = await app.getByTestId("panes-leaf-1").boundingBox();
  const split = await app.getByTestId("panes-split-s2").boundingBox();
  if (leaf1 && split && split.width > 0) {
    const frac = leaf1.width / split.width;
    if (frac < 0.15 || frac > 0.45) {
      throw new Error(`left pane fraction ${frac.toFixed(2)} not near 0.30 after ratio write`);
    }
  }

  await sleep(600); // let the debounced ratio write land before the host is killed

  console.log("ND_PANES_OK");
  await app.close();
  process.exit(0);
}

// --restore
for (const text of ["pane:1", "pane:2", "panes:2", "ratio:0.30"]) {
  await app.waitForText(text, { timeoutMs: T });
}
await expect(app.getByTestId("panes-split-s2")).toBeVisible();
await expect(app.getByTestId("panes-leaf-1")).toBeVisible();
await expect(app.getByTestId("panes-leaf-2")).toBeVisible();
await expect(app.getByTestId("pane-label-3")).toHaveCount(0);
console.log("ND_PANES_RESTORE_OK");
await app.close();
