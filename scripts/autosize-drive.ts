#!/usr/bin/env bun
// scripts/autosize-drive.ts [gtk|appkit] -- drives examples/counter/autosize-probe.tsx
// and examples/counter/main.tsx via @nativedesktop/test. Acceptance for sizing
// a window from its content: a <window> that names no defaultWidth/
// defaultHeight opens at its root's natural size (GTK's rule, which AppKit has
// none of), while a window that names both still opens at exactly those.
// Prints ND_AUTOSIZE_OK on success.
import { launchApp, findMatchingNode } from "../packages/test/src/index.ts";
import type { Backend } from "@nativedesktop/host";

const backend = process.argv[2] as Backend | undefined;

// The probe's two blocks carry explicit minWidth/minHeight, so the root's
// natural size is stated by the fixture rather than measured off font metrics:
// 520 wide, 240 + 8 spacing + 120 tall.
const CONTENT_WIDTH = 520;
const CONTENT_HEIGHT = 368;

function fail(msg: string): never {
  console.error(`FAIL: ${msg}`);
  process.exit(1);
}

const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));

{
  const app = await launchApp({ entry: "examples/counter/autosize-probe.tsx", backend });
  try {
    await app.waitForText("Sized from content", { timeoutMs: 8000 });
    // The window is sized on the settling pass after the root attaches.
    await sleep(500);
    const tree = (await app.tree()).root;
    const root = findMatchingNode(tree, (n) => n.testID === "autosize-root")?.geometry;
    const win = findMatchingNode(tree, (n) => n.type === "Window")?.geometry;
    if (!root || !win) fail("autosize probe: no window or root geometry");
    if (root.w !== CONTENT_WIDTH || root.h !== CONTENT_HEIGHT) {
      fail(`autosize probe: root is ${root.w}x${root.h}, expected ${CONTENT_WIDTH}x${CONTENT_HEIGHT}`);
    }
    // The window has to be at least the content plus the margins the root is
    // pinned with; the exact chrome is the platform's.
    if (win.w < root.w || win.h < root.h) {
      fail(`autosize probe: window ${win.w}x${win.h} is smaller than its content ${root.w}x${root.h}`);
    }
    // 480x320 is the schema fallback an undeclared window used to open at.
    if (win.w <= 480) fail(`autosize probe: window is still ${win.w} wide, the undeclared fallback`);
    console.log(`ND_AUTOSIZE_CONTENT_OK window ${win.w}x${win.h} for a ${root.w}x${root.h} root`);
  } finally {
    await app.close();
  }
}

{
  const app = await launchApp({ entry: "examples/counter/main.tsx", backend });
  try {
    await app.waitForText("Clicks:", { timeoutMs: 8000 });
    await sleep(500);
    const win = findMatchingNode((await app.tree()).root, (n) => n.type === "Window")?.geometry;
    if (!win) fail("counter: no window geometry");
    if (win.w !== 480 || win.h !== 320) {
      fail(`counter: declared 480x320 opened at ${win.w}x${win.h}`);
    }
    console.log(`ND_AUTOSIZE_DECLARED_OK declared 480x320 still wins`);
  } finally {
    await app.close();
  }
}

console.log(`ND_AUTOSIZE_OK backend=${backend ?? "appkit"}`);
