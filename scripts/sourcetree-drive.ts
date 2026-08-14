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
// Every wait in this drive is a UI round trip that settles in well under a
// second on an idle machine. The budget is env-overridable because the same
// drive runs inside scripts/browser-gate.sh, right after a full build on a box
// that may be shared, where 3s is not headroom.
const T = Number(process.env.ND_DRIVE_TIMEOUT_MS ?? 3000);

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
  // Rows carry their own identity, not just an index: a drive names a row.
  if (byTestId.get("st-run-1")!.id !== "run-1") throw new Error(`st-run-1 row id=${byTestId.get("st-run-1")!.id}, want run-1`);
  console.log(`ND_ST_TREE_OK role=tree rows=${rows.length} testIDs and ids present`);

  // iconData: raw image bytes where a freedesktop icon name cannot reach (a
  // favicon). Only the capture can prove the pixels, so this asserts the row
  // survived the decode and leaves the pixels to the screenshot legs below.
  if (!byTestId.has("st-run-2")) throw new Error("st-run-2 (the iconData row) missing from getTree rows");
  console.log("ND_ST_ICONDATA_OK row with iconData rendered");

  // ---- leg 2: handshake manifest + activation replay ------------------------
  await app.waitForText("caps present=true nope=false sourcetree=true", { timeoutMs: T });
  // The host records the launch activation state and replays it right after
  // HelloAck; a background spawn legitimately starts inactive, so first assert
  // only that the replay landed, then frontmost the process (works for both
  // backends; nd-hello runs via Quartz on macOS) and wait for the live flip.
  // The replay frame races the child's FIRST render (under load it lands
  // after commit 0) and the readout only recomputes on a re-render: poke one
  // with a selection round-trip before asserting.
  await app.setValue("st-tree", "run-1");
  await app.waitForText("replay=yes", { timeoutMs: T });
  await app.setValue("st-tree", "");
  await app.waitForText("sel (none)", { timeoutMs: T });
  // The live flip needs the process frontmost, which only macOS can be asked
  // for; a headless weston seat never activates a window, so the GTK leg
  // asserts the replay and stops there rather than failing on a missing
  // osascript.
  if (app.backend === "appkit") {
    const frontmost = Bun.spawnSync([
      "osascript", "-e",
      `tell application "System Events" to set frontmost of (first application process whose unix id is ${app.pid}) to true`,
    ]);
    if (frontmost.exitCode !== 0) throw new Error(`osascript frontmost failed: ${frontmost.stderr.toString()}`);
    await app.waitForText("active true replay=yes", { timeoutMs: T * 2 });
    console.log("ND_ST_CAPS_OK hasCommand true/false + hasWidget + isActive replay/live-flip verified");
  } else {
    console.log("ND_ST_CAPS_OK hasCommand true/false + hasWidget + isActive replay verified (no live flip: headless has no seat)");
  }

  // ---- leg 3: selection by node id (setValue -> selectionChanged -> a11y) ---
  await app.setValue("st-tree", "run-1");
  await app.waitForText("sel run-1", { timeoutMs: T });
  await app.waitForValue("st-tree", "run-1", { timeoutMs: T });
  console.log("ND_ST_SELECT_OK selectionChanged {nodeId} + a11y value by id");

  // ---- leg 3b: selectedId "" clears the selection ---------------------------
  // GTK regression: unselectAll is a documented no-op in browse mode, so the
  // clear path goes through selectRow(null). Re-select before leg 4, whose
  // semantic click activates the selected row.
  await app.setValue("st-tree", "");
  await app.waitForText("sel (none)", { timeoutMs: T });
  await app.setValue("st-tree", "run-1");
  await app.waitForText("sel run-1", { timeoutMs: T });
  console.log('ND_ST_CLEAR_OK setValue("") deselected; re-select landed');

  // ---- leg 4: semantic click activates the selected row ---------------------
  await app.click("st-tree");
  await app.waitForText("act run-1", { timeoutMs: T });
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
  const revealDeadline = Date.now() + T;
  for (;;) {
    try {
      await app.setValue("st-tree", "run-old");
      break;
    } catch (err) {
      if (Date.now() >= revealDeadline) throw err;
      await new Promise((r) => setTimeout(r, 100));
    }
  }
  await app.waitForText("sel run-old", { timeoutMs: T });
  console.log("ND_ST_EXPAND_PROP_OK collapsed shelf hides rows; expanding reveals them");

  // ---- leg 5b: semantic row action (both backends) ---------------------------
  // click {testId: <row testID>, action} dispatches the named trailing action
  // without a real pointer, the path GTK needs (no input synthesis) and the
  // one CanaryOrchestrator-style e2e flows use for row-level buttons.
  await app.click({ testId: "st-run-1", action: "close-run" });
  await app.waitForText("action close-run@run-1", { timeoutMs: T });
  // An action the node does not declare must fail loudly, not silently no-op.
  let undeclaredRejected = false;
  try {
    await app.click({ testId: "st-run-1", action: "new-run" });
  } catch {
    undeclaredRejected = true;
  }
  if (!undeclaredRejected) throw new Error("click(st-run-1, action new-run) should have failed: node does not declare it");
  console.log("ND_ST_ROWACTION_OK click {testId, action} dispatched actionClicked; undeclared action rejected");

  // ---- leg 6: native gestures (AppKit only; GTK4 cannot synthesize input) ---
  if (app.backend === "appkit") {
    const g = (await app.mustFind("st-tree")).geometry;
    if (!g) throw new Error("st-tree has no geometry");
    // Row 0 is proj-nd (26pt-ish single-line source rows). A real click both
    // selects it and makes the outline first responder for the keys below.
    const row0y = g.y + 14;
    await app.rpc.call("pointer", { phase: "down", x: g.x + 80, y: row0y });
    await app.rpc.call("pointer", { phase: "up", x: g.x + 80, y: row0y });
    await app.waitForText("sel proj-nd", { timeoutMs: T });
    console.log("ND_ST_POINTER_SELECT_OK pointer click selected proj-nd");

    await app.keys("left"); // collapse the selected parent
    await app.waitForText("expand collapsed:proj-nd", { timeoutMs: T });
    await app.keys("right"); // expand it again
    await app.waitForText("expand expanded:proj-nd", { timeoutMs: T });
    console.log("ND_ST_EXPAND_EVENT_OK nodeCollapsed/nodeExpanded from native disclosure keys");

    // Trailing action button on row 0 ("New Run", labeled inline button at
    // the row's right edge; actionVisibility "always" keeps it clickable).
    const actionX = g.x + g.w - 45;
    await app.rpc.call("pointer", { phase: "down", x: actionX, y: row0y });
    await app.rpc.call("pointer", { phase: "up", x: actionX, y: row0y });
    await app.waitForText("action new-run@proj-nd", { timeoutMs: T });
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
