#!/usr/bin/env bun
// scripts/keys-menu-drive.ts — asserts the M16 `keys` RPC drives native menu
// key equivalents: cmd+n against examples/notes must run File > New Note
// (the same handler scripts/notes-drive.ts exercises by clicking the menu
// item), observed as note-list's itemCount incrementing.
import { AutomationClient } from "@nativedesktop/test";
import type { JsonNode, GetTreeResult } from "../packages/react/src/generated/rpc.ts";

function find(node: JsonNode, testID: string): JsonNode | null {
  if (node.testID === testID) return node;
  for (const child of node.children) {
    const found = find(child, testID);
    if (found) return found;
  }
  return null;
}

const client = await AutomationClient.connect();
const t0 = (await client.call("getTree")) as GetTreeResult;
const list0 = find(t0.root, "note-list");
if (!list0) throw new Error("note-list not found");
const before = list0.itemCount ?? 0;

await client.call("keys", { keys: "cmd+n" });

let after = before;
for (let i = 0; i < 30; i++) {
  const t = (await client.call("getTree")) as GetTreeResult;
  after = find(t.root, "note-list")?.itemCount ?? before;
  if (after === before + 1) break;
  await new Promise((r) => setTimeout(r, 100));
}
if (after !== before + 1) throw new Error(`note-list itemCount ${before} -> ${after}, want ${before + 1}`);
console.log(`ND_KEYS_MENU_OK note-list itemCount ${before} -> ${after} via cmd+n key equivalent`);
client.close();
