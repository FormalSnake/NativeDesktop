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
  "linkHover",
  "contextMenu",
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

const ran = CHECKS.length - skips.length;
console.log(`ND_WEBVIEW2_OK ${ran}/${CHECKS.length} webview checks passed (${skips.length} skipped)`);
client.close();
