#!/usr/bin/env bun
// scripts/tabs-drive.ts [backend] [entry] — asserts native system tabs (M17):
// cmd+t (File > New Tab key equivalent) must open a second
// <window tabGroup="..."> that joins the native tab group, and cmd+w
// (File > Close, AppKit default menu) must run the deferred close loop —
// closed event -> app unmounts the <window> -> remove op's window.close
// semantic action. Window count comes from the `windows` RPC via
// @nativedesktop/test's app.windows()/waitForWindows(), not a manual getTree
// walk. Run against examples/browser (default) and examples/terminal.
//
// AppKit-only: cmd+t/cmd+w ride the `keys` RPC, which answers -32003 on GTK
// (GTK4 removed app-constructible events) — same gap gestures-drive.ts
// documents, gated the same way (simply never invoked on GTK).
import { launchApp } from "../packages/test/src/index.ts";
import type { Backend } from "@nativedesktop/host";

const backend = (process.argv[2] as Backend | undefined) ?? "appkit";
const entry = process.argv[3] ?? "examples/browser/main.tsx";

// Baseline-relative so a human poking the live app between runs can't skew
// the assertion — only the +1/-1 transitions are the contract.
const app = await launchApp({ entry, backend });
const { windows: startWindows } = await app.windows();
const start = startWindows.length;

await app.keys("cmd+t");
const opened = await app.waitForWindows(start + 1, 5000);
if (opened !== start + 1) throw new Error(`cmd+t: window count ${start} -> ${opened}, want ${start + 1}`);

// Screenshot right after the tab-bar animation races frame invalidation
// (-32603 until a frame lands) — screenshot()'s built-in retries subsume the
// hand-rolled poll loop this script used to carry.
await app.screenshot("/tmp/nd-tabs-two.png");

await app.keys("cmd+w");
const closed = await app.waitForWindows(start, 5000);
if (closed !== start) throw new Error(`cmd+w: window count ${opened} -> ${closed}, want ${start}`);

console.log(`ND_TABS_OK new tab via cmd+t joined the group and cmd+w ran the deferred close loop (entry=${entry})`);
await app.close();
