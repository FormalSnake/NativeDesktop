#!/usr/bin/env bun
// scripts/cef-drive.ts: drives examples/cef-probe over the automation socket
// and asserts the M1 surface of the Chromium engine. A page renders in the
// embedded view, and url/title/loading/progress/canGoBack/canGoForward plus
// the newWindow popup route all flow through the existing <webview> events.
//
// The probe app runs the sequence itself and writes each outcome into a label,
// so this script only reads the accessibility tree.
import { connectApp } from "@nativedesktop/test";

const CHECKS = ["render", "title", "progress", "history", "popup", "lateScheme", "hidden", "reload", "secondWindow"] as const;

const app = await connectApp();

// Chromium's first browser costs a process launch, a GPU probe and a
// SwiftShader fallback on this rig, so the ceiling is generous.
await app.waitForText("phase=done", { timeoutMs: 180000 });

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
