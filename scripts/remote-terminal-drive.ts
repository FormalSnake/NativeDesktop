#!/usr/bin/env bun
// scripts/remote-terminal-drive.ts — drives the <terminal remote> example over
// the automation socket against scripts/remote-terminal-fake-server.ts.
//
// Asserts: the terminal node is present; connectionState reached ATTACHED; the
// scripted title/bell/exit events were observed (surfaced into labels); the
// FLAG_RESET snapshot was fed (its title "snapshot-ready" arrives only after
// the reset); and a screenshot renders with non-zero dimensions (the banner
// bytes painted — the terminal grid itself has no getTree text).
import { AutomationClient } from "../packages/mcp/src/socket.ts";

interface TreeNode {
  ref: number;
  type: string;
  testID: string | null;
  text: string | null;
  children: TreeNode[];
}
interface GetTreeResult { coordinateSpace: string; root: TreeNode }

function find(node: TreeNode, pred: (n: TreeNode) => boolean): TreeNode | null {
  if (pred(node)) return node;
  for (const c of node.children) {
    const f = find(c, pred);
    if (f) return f;
  }
  return null;
}

const outPng = process.env.ND_SHOT_PATH ?? "/tmp/remote-terminal-shot.png";
const client = await AutomationClient.connect();

const tree = (await client.call("getTree")) as GetTreeResult;
const term = find(tree.root, (n) => n.testID === "remote-term" || n.type === "terminal");
if (!term) throw new Error("terminal node not found in getTree");

async function waitText(needle: string): Promise<void> {
  const res = (await client.call("waitFor", {
    condition: { textContains: needle },
    timeoutMs: 8000,
  })) as { matched: boolean };
  if (!res.matched) throw new Error(`timed out waiting for ${JSON.stringify(needle)}`);
}

// connectionState → ATTACHED (surfaced as `conn: attached`).
await waitText("conn: attached");
// onTitleChanged for the post-reset snapshot title — arrives only after the
// FLAG_RESET frame cleared the grid and its bytes were fed (proves reset).
await waitText("title: snapshot-ready");
// onBell.
await waitText("bells: 1");
// onExited (code 0).
await waitText("exit: 0");

const shot = (await client.call("screenshot", { path: outPng })) as { path: string; width: number; height: number };
if (shot.width <= 0 || shot.height <= 0) throw new Error("screenshot has no dimensions");

console.log(`REMOTE_DRIVE_OK term=#${term.ref} attached title bell exit reset png=${shot.path} ${shot.width}x${shot.height}`);
client.close();
