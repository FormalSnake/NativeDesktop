#!/usr/bin/env bun
// scripts/harness-drive.ts [gtk|appkit] — exercises @nativedesktop/test's own
// mechanics (not app assertions): launch, restart, close, screenshot floors,
// and killAll leaving no orphaned host process. Target is examples/counter,
// the smallest example in the repo.
import { existsSync } from "node:fs";
import { launchApp, killAll } from "../packages/test/src/index.ts";
import type { Backend } from "@nativedesktop/host";

const backend = process.argv[2] as Backend | undefined;
const shotDir = process.env.ND_SHOT_DIR ?? "/tmp";

function isAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

// 1. launch — ready markers observed, socket connected, tree reachable.
const app = await launchApp({ entry: "examples/counter/main.tsx", backend });
const firstPid = app.pid;
if (!isAlive(firstPid)) throw new Error("launchApp returned but the host process isn't running");

const clicksLabel = await app.mustFind("clicks-label");
if (clicksLabel.text !== "Clicks: 0") throw new Error(`clicks-label=${clicksLabel.text}, want "Clicks: 0"`);
console.log(`ND_HARNESS_LAUNCH_OK pid=${firstPid} backend=${app.backend}`);

// 2. actions round-trip through the harness's target normalization: string
// testID, then a raw ref, land the same click.
await app.click("increment-button");
await app.waitForText("Clicks: 1", { timeoutMs: 3000 });
const btn = await app.mustFind("increment-button");
await app.click(btn.ref);
await app.waitForText("Clicks: 2", { timeoutMs: 3000 });
console.log("ND_HARNESS_TARGET_OK string testID and numeric ref both resolved");

// 3. screenshot floors — retries + minBytes/minHeight, with the pngSize()
// parse baked into the result.
const shot = await app.screenshot(`${shotDir}/harness-drive.png`, { minHeight: 100, minBytes: 200 });
if (shot.width <= 0 || shot.height <= 0) throw new Error(`screenshot has no dimensions: ${JSON.stringify(shot)}`);
if (!existsSync(shot.path)) throw new Error(`screenshot path missing: ${shot.path}`);
console.log(`ND_HARNESS_SCREENSHOT_OK ${shot.path} ${shot.width}x${shot.height}`);

// A floor set above what the real screenshot delivers must fail loudly
// rather than silently return an under-sized image.
let floorRejected = false;
try {
  await app.screenshot(`${shotDir}/harness-drive-toobig.png`, { minHeight: 100_000, retries: 2 });
} catch {
  floorRejected = true;
}
if (!floorRejected) throw new Error("screenshot with an impossible minHeight floor should have thrown");
console.log("ND_HARNESS_SCREENSHOT_FLOOR_OK an impossible floor rejects instead of returning a bad shot");

// 4. restart — same AppHandle, a fresh process, state reset to the app's
// initial render (clicks back to 0), old pid gone.
await app.restart();
const secondPid = app.pid;
if (secondPid === firstPid) throw new Error("restart() did not spawn a new process");
if (isAlive(firstPid)) throw new Error(`restart() left the old pid ${firstPid} running`);
await app.waitForText("Clicks: 0", { timeoutMs: 5000 });
console.log(`ND_HARNESS_RESTART_OK pid ${firstPid} -> ${secondPid}, state reset`);

// 5. close — graceful shutdown, no leftover process.
await app.close();
if (isAlive(secondPid)) throw new Error(`close() left pid ${secondPid} running`);
console.log("ND_HARNESS_CLOSE_OK");

// 6. killAll — a second app registered, then torn down from module scope
// (the path a thrown assertion mid-test takes) leaves no orphan.
const app2 = await launchApp({ entry: "examples/counter/main.tsx", backend });
const orphanPid = app2.pid;
killAll();
for (let i = 0; i < 30 && isAlive(orphanPid); i++) await new Promise((r) => setTimeout(r, 100));
if (isAlive(orphanPid)) throw new Error(`killAll() left pid ${orphanPid} running`);
console.log(`ND_HARNESS_KILLALL_OK pid=${orphanPid} gone after killAll()`);

console.log(`ND_HARNESS_OK backend=${backend ?? "(default)"} launch/restart/close/screenshot-floors/killAll all verified`);
