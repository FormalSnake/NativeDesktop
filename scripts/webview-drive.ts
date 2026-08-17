#!/usr/bin/env bun
// scripts/webview-drive.ts — drives examples/webview-probe over the automation
// socket and asserts the browser/extension surface of <webview>: user scripts
// (document_start ordering, isolated worlds, targeted removal), script
// messages from both the page and an isolated world, world-scoped
// executeJavaScript, a custom URI scheme served by the app, cookie
// set/get/delete, per-view profile isolation, favicons, find-in-page, TLS
// state on a plain-http fixture, audio mute state and session save/restore.
//
// The probe app runs the whole sequence itself and writes each outcome into a
// label, so this script only reads the accessibility tree — no app-specific
// RPC, no cooperation beyond testIDs. Checks the engine cannot express
// headlessly report "skip: <reason>" and are listed, not failed.
import { AutomationClient, findNode } from "@nativedesktop/test";
import type { GetTreeResult, JsonNode } from "@nativedesktop/test";

const CHECKS = [
  "userScriptStart",
  "worldIsolation",
  "removeUserScript",
  "scriptMessagePage",
  "scriptMessageWorld",
  "scheme",
  "cookies",
  "profileIsolation",
  "favicon",
  "find",
  "security",
  "audio",
  "session",
  "download",
  "focus",
  "linkHover",
  "contextMenu",
  "contextMenuItems",
] as const;

function mustFind(tree: JsonNode, testID: string): JsonNode {
  const node = findNode(tree, testID);
  if (!node) throw new Error(`${testID} not found in tree`);
  return node;
}

const client = await AutomationClient.connect();

// The probe drives real page loads, a custom-scheme round trip and several
// async cookie writes, so the ceiling is generous; the host polls its own tree
// at ~50ms, this is not a busy wait.
await client.call("waitFor", { condition: { textContains: "phase=done" }, timeoutMs: 120000 });

const tree = (await client.call("getTree")) as GetTreeResult;

const failures: string[] = [];
const skips: string[] = [];
for (const name of CHECKS) {
  const text = mustFind(tree.root, `chk-${name}`).text ?? "";
  const value = text.slice(text.indexOf("=") + 1);
  if (value.startsWith("ok")) {
    console.log(`ND_WEBVIEW_CHECK ${name}: ${value}`);
  } else if (value.startsWith("skip")) {
    skips.push(`${name}: ${value}`);
  } else {
    failures.push(`${name}: ${value}`);
  }
}

for (const skip of skips) console.log(`ND_WEBVIEW_SKIP ${skip}`);
if (failures.length > 0) {
  for (const failure of failures) console.error(`ND_WEBVIEW_FAIL ${failure}`);
  throw new Error(`${failures.length} webview check(s) failed`);
}

await client.call("screenshot", { path: process.env.ND_SHOT_PATH ?? "/tmp/nd-webview-probe.png" });

// ---------------------------------------------------------------------------
// Automation-side page vocabulary. Unlike the checks above, none of this asks
// the app for anything: webviewInfo / webviewEval / the page waitFor
// predicates read the engine directly, which is the whole point — a drive must
// work against an app that forwards no events.
const base = (mustFind(tree.root, "probe-base").text ?? "").slice("base=".length);
if (!base.startsWith("http://")) throw new Error(`probe-base label was ${JSON.stringify(base)}`);

const info = (await client.call("webviewInfo", { testId: "wv-main" })) as {
  ref: number;
  url: string | null;
  title: string | null;
  loading: boolean;
  canGoBack: boolean;
  canGoForward: boolean;
};
if (!info.url?.startsWith(base)) throw new Error(`webviewInfo.url was ${JSON.stringify(info.url)}, expected ${base}`);
if (info.title !== "ND Probe Fixture") throw new Error(`webviewInfo.title was ${JSON.stringify(info.title)}`);
if (!info.canGoBack) throw new Error("webviewInfo.canGoBack was false after several navigations");
console.log(`ND_WEBVIEW_CHECK webviewInfo: ok (${info.title} @ ${info.url})`);

const evaluated = (await client.call("webviewEval", {
  testId: "wv-main",
  code: "document.getElementById('p').textContent",
})) as { ok: boolean; value: string | null; error: string | null };
if (!evaluated.ok || !evaluated.value?.includes("needle one")) {
  throw new Error(`webviewEval returned ${JSON.stringify(evaluated)}`);
}
console.log("ND_WEBVIEW_CHECK webviewEval: ok");

// A world-scoped eval reaches the isolated world the probe injected into.
const inWorld = (await client.call("webviewEval", {
  testId: "wv-main",
  code: "window.__ndWorld",
  world: "probe",
})) as { ok: boolean; value: string | null };
if (!inWorld.ok || inWorld.value !== "world-ok") throw new Error(`world eval returned ${JSON.stringify(inWorld)}`);
console.log("ND_WEBVIEW_CHECK webviewEvalWorld: ok");

// A thrown exception is a RESULT, not an RPC error.
const threw = (await client.call("webviewEval", {
  testId: "wv-main",
  code: "throw new Error('probe-boom')",
})) as { ok: boolean; error: string | null };
if (threw.ok || !threw.error?.includes("probe-boom")) throw new Error(`throwing eval returned ${JSON.stringify(threw)}`);
console.log("ND_WEBVIEW_CHECK webviewEvalThrow: ok");

for (const [name, condition] of [
  ["urlContains", { testId: "wv-main", urlContains: new URL(base).port }],
  ["pageTitleContains", { testId: "wv-main", pageTitleContains: "Probe Fixture" }],
  ["pageTextContains", { testId: "wv-main", pageTextContains: "needle two" }],
] as const) {
  await client.call("waitFor", { condition, timeoutMs: 10000 });
  console.log(`ND_WEBVIEW_CHECK ${name}: ok`);
}

// The focus command's real assertion: the a11y probe, read over the socket
// rather than from anything the app told us.
await client.call("waitFor", { condition: { testId: "wv-main", state: "focused" }, timeoutMs: 5000 });
console.log("ND_WEBVIEW_CHECK focusCommand: ok");

const ran = CHECKS.length - skips.length;
console.log(`ND_WEBVIEW2_OK ${ran}/${CHECKS.length} webview checks passed (${skips.length} skipped), page vocabulary verified`);
client.close();
