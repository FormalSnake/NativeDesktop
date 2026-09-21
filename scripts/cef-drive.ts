#!/usr/bin/env bun
// scripts/cef-drive.ts: drives examples/cef-probe over the automation socket
// and asserts the M1 surface of the Chromium engine. A page renders in the
// embedded view, and url/title/loading/progress/canGoBack/canGoForward plus
// the newWindow popup route all flow through the existing <webview> events.
//
// The probe app runs the sequence itself and writes each outcome into a label,
// so this script only reads the accessibility tree.
import { connectApp } from "@nativedesktop/test";

const CHECKS = ["render", "title", "progress", "history", "popup", "lateScheme", "hidden", "reload", "secondWindow", "extensions", "extensionsChanged", "runtimeExtensions", "uninstallExtension", "chromeDialog"] as const;

const app = await connectApp();

// Chromium's first browser costs a process launch, a GPU probe and a
// SwiftShader fallback on this rig, so the ceiling is generous.
await app.waitForText("phase=done", { timeoutMs: 180000 });

// Chrome asks before it removes an extension, in a dialog of its own that the
// engine moves over the view and reports as `chromeDialog`. Nothing in the app
// can answer a Views dialog, so the click is a real X one, aimed at the
// bottom-right button using the geometry the probe wrote down.
let dialog = "";
for (let i = 0; i < 80; i++) {
  dialog = (await app.getByTestId("chk-chromeDialog").textContent()) ?? "";
  if (/=(ok|skip|fail)/.test(dialog)) break;
  await Bun.sleep(250);
}
const box = dialog.match(/\((-?\d+),(-?\d+) (\d+)x(\d+)\)/);
if (box) {
  const [, x, y, w, h] = box.map(Number);
  // Bottom-right of Chrome's two-button row, measured against the 448x137
  // "Remove …?" dialog.
  // The dialog was moved onto the view a moment ago and Views does not take a
  // click until it has laid out at the new place.
  await Bun.sleep(1500);
  Bun.spawnSync(["xdotool", "mousemove", String(x + w - 62), String(y + h - 39), "click", "1"], {
    env: { ...process.env, DISPLAY: process.env.DISPLAY ?? ":96" },
  });
  console.log(`  chromeDialogClick: ${x + w - 62},${y + h - 39}`);
  try {
    await app.waitForText("uninstallExtension=ok", { timeoutMs: 20000 });
  } catch {
    // Reported as a failed check below, with the label's own text.
  }
}

const failures: string[] = [];
for (const name of CHECKS) {
  const text = (await app.getByTestId(`chk-${name}`).textContent()) ?? "";
  const value = text.slice(text.indexOf("=") + 1);
  if (!value.startsWith("ok") && !value.startsWith("skip")) failures.push(`${name}: ${value}`);
  console.log(`  ${name}: ${value}`);
}

// The live view state, straight off the engine rather than off the app's own
// event bookkeeping: `webviewInfo` reads what the CEF handlers last pushed.
const info = await app.rpc.call("webviewInfo", { testId: "wv" });
console.log(`  webviewInfo: ${JSON.stringify(info)}`);
if (!info.url?.startsWith("http://127.0.0.1:")) {
  failures.push(`webviewInfo.url is ${JSON.stringify(info.url)}, want the fixture origin`);
}

if (process.env.ND_SHOT_PATH) {
  // The host's own screenshot cannot rasterize a live webview (it degrades to
  // a placeholder over the rect, see src/gtk/backend.zig), so the picture that
  // proves a page rendered is taken at the X server instead. This one is a
  // liveness check, not evidence, so a frame that has not landed yet is a note
  // rather than a failure.
  try {
    await app.screenshot(process.env.ND_SHOT_PATH);
  } catch (error) {
    console.log(`  host screenshot skipped: ${(error as Error).message}`);
  }
}

if (failures.length > 0) {
  console.error(`ND_CEF_FAIL ${failures.length} check(s) failed:`);
  for (const f of failures) console.error(`  - ${f}`);
  process.exit(1);
}

console.log(`ND_CEF_M1_OK ${CHECKS.length} checks passed on the chromium engine`);
await app.close();
