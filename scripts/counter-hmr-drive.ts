#!/usr/bin/env bun
// scripts/counter-hmr-drive.ts: Solid hot reload under ND_DEV=1
// (`bun --hot`). Runs a copy of examples/counter/main.tsx and edits it
// while the app is up:
//   1. an edit to ClicksLabel patches that component alone: the window keeps
//      its native ref, App's clicks signal keeps its value, the host tree
//      keeps its node count;
//   2. a second edit to the same component patches again (the runtime keeps
//      patching the first-mounted proxies);
//   3. removing a component, which the refresh runtime cannot patch, remounts
//      the whole tree: one window still, the same node count, no node left
//      behind.
// ND_HOST_BINARY picks the host (e.g. swift/.build/release/NDShell).
import { copyFileSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";
import type { JsonNode } from "../packages/core/src/generated/rpc.ts";
import { launchApp, expect, poll } from "../packages/test/src/index.ts";

const exampleDir = join(import.meta.dir, "..", "examples", "counter");
// Inside the example so its package.json (and so the Solid preload) applies.
const dir = mkdtempSync(join(exampleDir, ".hmr-"));
const entry = join(dir, "main.tsx");
copyFileSync(join(exampleDir, "main.tsx"), entry);

function edit(from: string, to: string): void {
  const src = readFileSync(entry, "utf8");
  if (!src.includes(from)) throw new Error(`hmr drive: "${from}" not in ${entry}`);
  writeFileSync(entry, src.replace(from, to));
}

const count = (n: JsonNode): number => 1 + n.children.reduce((s, c) => s + count(c), 0);

let app: Awaited<ReturnType<typeof launchApp>> | undefined;
try {
  app = await launchApp({ entry: relative(process.cwd(), entry), dev: true, hostBinary: process.env.ND_HOST_BINARY });
  const label = app.getByTestId("clicks-label");
  await expect(label).toHaveText("Clicks: 0");
  await app.getByTestId("increment-button").click();
  await app.getByTestId("increment-button").click();
  await expect(label).toHaveText("Clicks: 2");
  await expect(app.getByTestId("badge-label")).toHaveText("ready:loading-resolved");
  const [win] = (await app.windows()).windows;
  const nodes = count((await app.tree()).root);

  edit("text={`Clicks: ${props.clicks}`}", "text={`Taps: ${props.clicks}`}");
  await expect(label).toHaveText("Taps: 2", { timeout: 10_000 });
  let windows = (await app.windows()).windows;
  if (windows.length !== 1 || windows[0]!.ref !== win!.ref) throw new Error(`window replaced: ${JSON.stringify(windows)}`);
  if (count((await app.tree()).root) !== nodes) throw new Error("node count changed after a component patch");

  edit("text={`Taps: ${props.clicks}`}", "text={`Presses: ${props.clicks}`}");
  await expect(label).toHaveText("Presses: 2", { timeout: 10_000 });
  await app.getByTestId("increment-button").click();
  await expect(label).toHaveText("Presses: 3");

  // Inlining ClicksLabel into App removes a registered component.
  const src = readFileSync(entry, "utf8");
  writeFileSync(
    entry,
    src
      .replace(/function ClicksLabel[\s\S]*?\n}\n/, "")
      .replace("<ClicksLabel clicks={clicks()} />", '<label testID="clicks-label" text={`Inline: ${clicks()}`} />'),
  );
  await expect(label).toHaveText("Inline: 0", { timeout: 10_000 });
  windows = (await poll(() => app.windows(), (r) => r.windows.length === 1, { timeoutMs: 5000 })).windows;
  await expect(app.getByTestId("badge-label")).toHaveText("ready:loading-resolved");
  const after = count((await app.tree()).root);
  if (after !== nodes) throw new Error(`node count ${nodes} before, ${after} after the remount`);

  console.log(
    `ND_COUNTER_HMR_OK patch kept window ref=${win!.ref} and clicks, remount kept ${after} nodes and ${windows.length} window`,
  );
} finally {
  await app?.close();
  rmSync(dir, { recursive: true, force: true });
}
