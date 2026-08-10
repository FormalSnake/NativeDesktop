#!/usr/bin/env bun
// scripts/errors-drive.ts: drives examples/errors over the automation socket.
//
// Modes:
//   --survive:       click reject-async (non-fatal unhandled rejection), then
//                    bump -> waitFor "Count: 1" (reconciler still commits),
//                    then throw-caught -> waitFor the boundary fallback ->
//                    ERRORS_SURVIVE_OK. Host-log assertions (the
//                    ND_RUNTIME_ERROR_NONFATAL prints, no exit, no overlay)
//                    live in scripts/headless-errors.sh.
//   --fatal:         click throw-sync (uncaughtException, fatal by default),
//                    print ERRORS_FATAL_CLICKED. The child exits; the shell
//                    waits on ND_CHILD_EXITED + ND_OVERLAY_SHOWN.
//   --overlay-check: assert nd-overlay-error shows the sync throw's message,
//                    NOT the earlier non-fatal one (the no-stash rule) ->
//                    ERRORS_OVERLAY_OK.
import { AutomationClient } from "../packages/mcp/src/socket.ts";

interface TreeNode {
  ref: number;
  type: string;
  testID: string | null;
  text: string | null;
  visible: boolean;
  geometry: { x: number; y: number; w: number; h: number } | null;
  children: TreeNode[];
}

interface GetTreeResult {
  coordinateSpace: string;
  root: TreeNode;
}

function find(node: TreeNode, testID: string): TreeNode | null {
  if (node.testID === testID) return node;
  for (const child of node.children) {
    const found = find(child, testID);
    if (found) return found;
  }
  return null;
}

async function click(client: AutomationClient, tree: GetTreeResult, testID: string): Promise<void> {
  const btn = find(tree.root, testID);
  if (!btn) throw new Error(`${testID} not found in tree`);
  const res = (await client.call("click", { ref: btn.ref })) as { ref: number; dispatched: boolean };
  if (!res.dispatched) throw new Error(`${testID} click did not dispatch`);
}

async function waitForText(client: AutomationClient, text: string, timeoutMs: number): Promise<void> {
  const waited = (await client.call("waitFor", {
    condition: { textContains: text },
    timeoutMs,
  })) as { matched: boolean };
  if (!waited.matched) throw new Error(`waitFor ${JSON.stringify(text)} did not match`);
}

const mode = process.argv.includes("--survive") ? "survive" : process.argv.includes("--fatal") ? "fatal" : "overlay-check";

const client = await AutomationClient.connect();

if (mode === "survive") {
  const tree = (await client.call("getTree")) as GetTreeResult;
  if (tree.coordinateSpace !== "logical-window-topleft") throw new Error("bad coordinate space");

  await click(client, tree, "reject-async");
  await click(client, tree, "bump");
  await waitForText(client, "Count: 1", 3000);

  await click(client, tree, "throw-caught");
  await waitForText(client, "caught: render-throw", 3000);
  const tree2 = (await client.call("getTree")) as GetTreeResult;
  const fallback = find(tree2.root, "boundary-fallback");
  if (!fallback?.text?.includes("render-throw")) {
    throw new Error(`boundary fallback missing or wrong: ${JSON.stringify(fallback?.text)}`);
  }

  console.log(`ERRORS_SURVIVE_OK fallback=${JSON.stringify(fallback.text)}`);
  client.close();
  process.exit(0);
}

if (mode === "fatal") {
  const tree = (await client.call("getTree")) as GetTreeResult;
  await click(client, tree, "throw-sync");
  console.log("ERRORS_FATAL_CLICKED");
  client.close();
  process.exit(0);
}

// overlay-check
const tree = (await client.call("getTree")) as GetTreeResult;
const errNode = find(tree.root, "nd-overlay-error");
if (!errNode?.text) throw new Error(`nd-overlay-error missing or empty: ${JSON.stringify(errNode)}`);
if (!errNode.text.includes("sync-throw")) {
  throw new Error(`overlay shows the wrong error (want sync-throw): ${JSON.stringify(errNode.text)}`);
}
if (errNode.text.includes("async-reject")) {
  throw new Error(`overlay leaked a stale non-fatal message: ${JSON.stringify(errNode.text)}`);
}
console.log(`ERRORS_OVERLAY_OK error=${JSON.stringify(errNode.text)}`);
client.close();
