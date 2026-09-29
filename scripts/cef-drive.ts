#!/usr/bin/env bun
// scripts/cef-drive.ts: drives examples/cef-probe over the automation socket
// and asserts the M1 surface of the Chromium engine. A page renders in the
// embedded view, and url/title/loading/progress/canGoBack/canGoForward plus
// the newWindow popup route all flow through the existing <webview> events.
//
// The probe app runs the sequence itself and writes each outcome into a label,
// so this script only reads the accessibility tree.
import { connectApp } from "@nativedesktop/test";

const CHECKS = ["render", "title", "progress", "history", "popup", "lateScheme", "hidden", "reload", "secondWindow", "closedBrowser", "removedNode", "extensions", "twoRegistryViews", "extensionsChanged", "runtimeActionState", "actionClick", "installExtensionError", "runtimeExtensions", "uninstallExtension", "uninstallSilent"] as const;

const app = await connectApp();

// Commands sent to a removed view, and two views on chrome://extensions, each
// once took the host down to where it answered nothing at all, so the host is
// timed the moment each of those legs has run.
const stalls: string[] = [];
for (const [leg, what] of [["removedNode", "commands to a removed view"], ["twoRegistryViews", "two views on chrome://extensions"]]) {
  for (let i = 0; i < 720; i++) {
    const text = (await app.getByTestId(`chk-${leg}`).textContent()) ?? "";
    if (/=(ok|skip|fail)/.test(text)) break;
    await Bun.sleep(250);
  }
  const started = performance.now();
  await app.tree();
  const ms = Math.round(performance.now() - started);
  console.log(`  treeAfter ${leg}: ${ms}ms`);
  if (ms > 1000) stalls.push(`getTree took ${ms}ms right after ${what}`);
}

// Chromium's first browser costs a process launch, a GPU probe and a
// SwiftShader fallback on this rig, so the ceiling is generous.
try {
  await app.waitForText("phase=done", { timeoutMs: 180000 });
} catch (error) {
  console.error(`ND_CEF_FAIL the probe never finished: ${(await app.getByTestId("probe-phase").textContent()) ?? "no phase"}`);
  for (const name of CHECKS) console.error(`  ${(await app.getByTestId(`chk-${name}`).textContent()) ?? name}`);
  throw error;
}

const failures: string[] = [...stalls];
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
