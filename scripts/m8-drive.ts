#!/usr/bin/env bun
// scripts/m8-drive.ts — drives the M8 HMR + crash-overlay demo over the automation socket.
//
// Three modes:
//   --hmr-check:  getTree -> find increment-button -> click x2 -> waitFor "Clicks: 2" ->
//                 print the label text -> M8_HMR_PRECHECK_OK (run BEFORE the fixture edit)
//   --hmr-verify: getTree -> assert clicks-label still reads "Clicks: 2" (state preserved)
//                 AND the button's own text reflects the edited label -> M8_HMR_OK
//                 (run AFTER the fixture edit)
//   default (crash leg): getTree -> assert nd-overlay-error present with non-empty text ->
//                 click nd-overlay-restart -> waitFor "Clicks:" (app re-mounted) -> M8_CRASH_OK
import { AutomationClient } from "@nativedesktop/test";

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

const mode = process.argv.includes("--hmr-check")
  ? "hmr-check"
  : process.argv.includes("--hmr-verify")
    ? "hmr-verify"
    : "crash";

const client = await AutomationClient.connect();

if (mode === "hmr-check") {
  const tree = (await client.call("getTree")) as GetTreeResult;
  if (tree.coordinateSpace !== "logical-window-topleft") throw new Error("bad coordinate space");
  const btn = find(tree.root, "increment-button");
  if (!btn) throw new Error("increment-button not found in tree");

  for (let i = 0; i < 2; i++) {
    const res = (await client.call("click", { ref: btn.ref })) as { ref: number; dispatched: boolean };
    if (!res.dispatched) throw new Error(`click ${i + 1} did not dispatch`);
  }

  const waited = (await client.call("waitFor", {
    condition: { textContains: "Clicks: 2" },
    timeoutMs: 3000,
  })) as { matched: boolean };
  if (!waited.matched) throw new Error("waitFor Clicks: 2 did not match");

  const tree2 = (await client.call("getTree")) as GetTreeResult;
  const label = find(tree2.root, "clicks-label");
  console.log(`M8_HMR_PRECHECK_OK label=${JSON.stringify(label?.text)}`);
  client.close();
  process.exit(0);
}

if (mode === "hmr-verify") {
  const tree = (await client.call("getTree")) as GetTreeResult;
  const label = find(tree.root, "clicks-label");
  const btn = find(tree.root, "increment-button");
  if (!label || label.text !== "Clicks: 2") {
    throw new Error(`state not preserved: clicks-label is ${JSON.stringify(label?.text)}, expected "Clicks: 2"`);
  }
  if (!btn || !btn.text?.includes("!")) {
    throw new Error(`edit did not land: increment-button text is ${JSON.stringify(btn?.text)}`);
  }
  console.log(`M8_HMR_OK label=${JSON.stringify(label.text)} button=${JSON.stringify(btn.text)}`);
  client.close();
  process.exit(0);
}

// crash leg (default)
const tree = (await client.call("getTree")) as GetTreeResult;
const errNode = find(tree.root, "nd-overlay-error");
if (!errNode || !errNode.text) {
  throw new Error(`nd-overlay-error missing or empty: ${JSON.stringify(errNode)}`);
}

const restartBtn = find(tree.root, "nd-overlay-restart");
if (!restartBtn) throw new Error("nd-overlay-restart not found (dev mode should expose it)");

const clicked = (await client.call("click", { ref: restartBtn.ref })) as { ref: number; dispatched: boolean };
if (!clicked.dispatched) throw new Error("restart click did not dispatch");

const waited = (await client.call("waitFor", {
  condition: { textContains: "Clicks:" },
  timeoutMs: 5000,
})) as { matched: boolean };
if (!waited.matched) throw new Error("app did not re-mount after restart");

console.log(`M8_CRASH_OK error=${JSON.stringify(errNode.text)}`);
client.close();
