#!/usr/bin/env bun
// scripts/sourcetree-drive.ts [gtk|appkit] — drives examples/sourcetree/main.tsx
// over the automation socket via @nativedesktop/test. Asserts the SourceTree
// wave checklist: rows/roles/testIDs in getTree, selectionChanged by id,
// rowActivated, expand/collapse honoring the controlled `expanded` flags,
// actionClicked {nodeId, actionId} + native disclosure events (AppKit
// pointer/keys leg; GTK cannot synthesize input, -32003), hasCommand/
// hasWidget from the handshake manifest, app.isActive() with host replay,
// and the AppKit `toolbar` structural class (screenshot). Prints
// ND_SOURCETREE_OK on success.
import { launchApp } from "../packages/test/src/index.ts";
import type { Backend } from "@nativedesktop/host";

const backend = process.argv[2] as Backend | undefined;
const shotDir = process.env.ND_SHOT_DIR ?? "/tmp";

const app = await launchApp({ entry: "examples/sourcetree/main.tsx", backend });
try {
  // ---- leg 1: getTree shape — role, itemCount, rows with testIDs ------------
  const tree = await app.mustFind("st-tree");
  if (tree.role !== "tree") throw new Error(`st-tree role=${tree.role}, want "tree"`);
  if (tree.itemCount !== 9) throw new Error(`st-tree itemCount=${tree.itemCount}, want 9`);
  const rows = tree.rows ?? [];
  const byTestId = new Map(rows.map((r) => [r.testID, r]));
  for (const id of ["st-sec-hosts", "st-host-mac", "st-proj-nd", "st-run-1", "st-run-old"]) {
    if (!byTestId.has(id)) throw new Error(`getTree rows missing testID ${id} (got: ${rows.map((r) => r.testID).join(",")})`);
  }
  if (byTestId.get("st-run-1")!.title !== "fix sidebar") throw new Error("st-run-1 row title mismatch");
  if (byTestId.get("st-run-1")!.badge !== "3") throw new Error("st-run-1 row badge mismatch");
  if (byTestId.get("st-host-mac")!.iconName !== "computer-symbolic") throw new Error("st-host-mac row iconName mismatch");
  console.log(`ND_ST_TREE_OK role=tree rows=${rows.length} testIDs present`);

  // ---- leg 2: handshake manifest + activation replay ------------------------
  await app.waitForText("caps present=true nope=false sourcetree=true", { timeoutMs: 3000 });
  // The host records the launch activation state and replays it right after
  // HelloAck; a background spawn legitimately starts inactive, so first assert
  // only that the replay landed, then frontmost the process (works for both
  // backends; nd-hello runs via Quartz on macOS) and wait for the live flip.
  await app.waitForText("replay=yes", { timeoutMs: 3000 });
  const frontmost = Bun.spawnSync([
    "osascript", "-e",
    `tell application "System Events" to set frontmost of (first application process whose unix id is ${app.pid}) to true`,
  ]);
  if (frontmost.exitCode !== 0) throw new Error(`osascript frontmost failed: ${frontmost.stderr.toString()}`);
  await app.waitForText("active true replay=yes", { timeoutMs: 5000 });
  console.log("ND_ST_CAPS_OK hasCommand true/false + hasWidget + isActive replay/live-flip verified");

  // ---- leg 3: selection by node id (setValue -> selectionChanged -> a11y) ---
  await app.setValue("st-tree", "run-1");
  await app.waitForText("sel run-1", { timeoutMs: 3000 });
  await app.waitForValue("st-tree", "run-1", { timeoutMs: 3000 });
  console.log("ND_ST_SELECT_OK selectionChanged {nodeId} + a11y value by id");

  // ---- leg 4: semantic click activates the selected row ---------------------
  await app.click("st-tree");
  await app.waitForText("act run-1", { timeoutMs: 3000 });
  console.log("ND_ST_ACTIVATE_OK rowActivated {nodeId}");

  // ---- leg 5: expansion honors the controlled `expanded` flags --------------
  // sec-settled starts collapsed: its child row does not exist natively, so
  // an id-addressed setValue must fail loudly rather than select nothing.
  let collapsedRejected = false;
  try {
    await app.setValue("st-tree", "run-old");
  } catch {
    collapsedRejected = true;
  }
  if (!collapsedRejected) throw new Error("setValue(run-old) inside a collapsed section should have failed");
  await app.click("st-settled-toggle"); // app state -> expanded flag -> rebuild
  // The click RPC returns before the toggle's JS round-trip lands the nodes
  // update, so retry until the revealed row is selectable.
  const revealDeadline = Date.now() + 3000;
  for (;;) {
    try {
      await app.setValue("st-tree", "run-old");
      break;
    } catch (err) {
      if (Date.now() >= revealDeadline) throw err;
      await new Promise((r) => setTimeout(r, 100));
    }
  }
  await app.waitForText("sel run-old", { timeoutMs: 3000 });
  console.log("ND_ST_EXPAND_PROP_OK collapsed shelf hides rows; expanding reveals them");

  // ---- leg 6: native gestures (AppKit only; GTK4 cannot synthesize input) ---
  if (app.backend === "appkit") {
    const g = (await app.mustFind("st-tree")).geometry;
    if (!g) throw new Error("st-tree has no geometry");
    // Row 0 is proj-nd (26pt-ish single-line source rows). A real click both
    // selects it and makes the outline first responder for the keys below.
    const row0y = g.y + 14;
    await app.rpc.call("pointer", { phase: "down", x: g.x + 80, y: row0y });
    await app.rpc.call("pointer", { phase: "up", x: g.x + 80, y: row0y });
    await app.waitForText("sel proj-nd", { timeoutMs: 3000 });
    console.log("ND_ST_POINTER_SELECT_OK pointer click selected proj-nd");

    await app.keys("left"); // collapse the selected parent
    await app.waitForText("expand collapsed:proj-nd", { timeoutMs: 3000 });
    await app.keys("right"); // expand it again
    await app.waitForText("expand expanded:proj-nd", { timeoutMs: 3000 });
    console.log("ND_ST_EXPAND_EVENT_OK nodeCollapsed/nodeExpanded from native disclosure keys");

    // Trailing action button on row 0 ("New Run", labeled inline button at
    // the row's right edge; actionVisibility "always" keeps it clickable).
    const actionX = g.x + g.w - 45;
    await app.rpc.call("pointer", { phase: "down", x: actionX, y: row0y });
    await app.rpc.call("pointer", { phase: "up", x: actionX, y: row0y });
    await app.waitForText("action new-run@proj-nd", { timeoutMs: 3000 });
    console.log("ND_ST_ACTION_OK actionClicked {nodeId, actionId}");

    const shot = await app.screenshot(`${shotDir}/sourcetree-appkit.png`, { minBytes: 2000 });
    console.log(`ND_ST_TOOLBAR_SHOT ${shot.path} ${shot.width}x${shot.height}`);
  } else {
    console.log("ND_ST_GESTURES_SKIP gtk: input synthesis unsupported (-32003); disclosure/action click paths verified on the appkit leg");
    const shot = await app.screenshot(`${shotDir}/sourcetree-gtk.png`, { minBytes: 2000 });
    console.log(`ND_ST_SHOT ${shot.path} ${shot.width}x${shot.height}`);
  }

  // Notification click needs a real user click on an OS banner; the data
  // correlation map is covered by packages/react/src/system.test.ts instead.
  console.log("ND_ST_NOTIFICATION_SKIP data echo covered by unit tests (OS banner click is not synthesizable)");

  console.log(`ND_SOURCETREE_OK backend=${app.backend}`);
} finally {
  await app.close();
}
