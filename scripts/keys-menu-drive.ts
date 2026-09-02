#!/usr/bin/env bun
// scripts/keys-menu-drive.ts — asserts the M16 `keys` RPC drives native menu
// key equivalents: cmd+n against examples/notes must run File > New Note
// (the same handler scripts/notes-drive.ts exercises by clicking the menu
// item), observed as note-list's itemCount incrementing.
import { connectApp, poll } from "../packages/test/src/index.ts";

const app = await connectApp();
const list = app.getByTestId("note-list");
const before = (await list.node()).itemCount ?? 0;

await app.keyboard.press("Meta+n");

// itemCount is a widget field rather than a11y state, so no waitFor
// predicate reaches it.
const after = await poll(
  async () => (await list.node()).itemCount ?? before,
  (count) => count === before + 1,
  { timeoutMs: 3000, intervalMs: 100 },
).catch(() => -1);
if (after !== before + 1) throw new Error(`note-list itemCount ${before} -> ${after}, want ${before + 1}`);
console.log(`ND_KEYS_MENU_OK note-list itemCount ${before} -> ${after} via cmd+n key equivalent`);
await app.close();
