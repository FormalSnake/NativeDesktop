#!/usr/bin/env bun
// scripts/popover-anchor-drive.ts [gtk|appkit] -- drives
// examples/popoveranchor/main.tsx via @nativedesktop/test. Acceptance for
// <popover anchorRef>: the popover is rendered through a portal into the
// off-window pool, so it has no tree parent to anchor on and `anchorRef` is
// the only thing that can place it. Prints ND_POPOVER_ANCHOR_OK on success.
//
// The two backends answer "is it placed?" with different evidence. AppKit
// presents the panel's content, so `visible` is the readable signal. GTK puts
// the panel in its own GdkSurface, which never maps under a headless
// compositor with no seat, so the readable signal is the geometry getTree
// reports for it: unplaced it has none at all (an unparented GtkPopover is
// unrooted, and compute_bounds fails), and placed its origin sits inside the
// trigger button. Closing differs too: `press` rides the `keys` RPC, which
// GTK answers -32003, so the GTK leg drops the ref while the panel is up and
// lets the resulting close do the work.
//
// Runs either way round: scripts/headless-popover.sh owns the host and hands
// the socket over in ND_AUTOMATION_SOCKET, and a bare
// `bun scripts/popover-anchor-drive.ts` launches one itself.
import { connectApp, findNode, launchApp, poll } from "../packages/test/src/index.ts";
import type { Backend } from "@nativedesktop/host";

const backend = process.argv[2] as Backend | undefined;
const gtk = process.env.ND_BACKEND === "gtk";
const attached = process.env.ND_AUTOMATION_SOCKET != null;
const T = 4000;

const app = attached ? await connectApp() : await launchApp({ entry: "examples/popoveranchor/main.tsx", backend });
const mustFind = async (testId: string) => {
  const node = findNode((await app.tree()).root, testId);
  if (!node) throw new Error(`${testId} not found in tree`);
  return node;
};
const body = async () => findNode((await app.tree()).root, "popover-body");
const shown = async () => {
  const b = await body();
  if (!b) return false;
  return gtk ? b.geometry != null : b.visible === true;
};
type Rect = { x: number; y: number; w: number; h: number };
const encloses = (r: Rect, p: Rect) => p.x >= r.x && p.x <= r.x + r.w && p.y >= r.y && p.y <= r.y + r.h;

try {
  const trigger = await mustFind("anchor-button");
  if (!trigger.geometry?.w) throw new Error("the trigger button never laid out");
  if (await shown()) throw new Error("the popover was already presenting before anything opened it");

  // ---- leg 1: the ref places a popover with no tree parent at all ----------
  await app.getByTestId("anchor-button").click();
  await app.waitForValue("open-label", "open", { timeoutMs: T });
  await poll(() => shown(), (v) => v, { timeoutMs: T });
  if (gtk) {
    // Placement, not just presence: the panel's origin has to land on the node
    // `anchorRef` names and not on the other button in the same column, which
    // is what a fallback to "the first thing in the window" would give.
    const panel = (await mustFind("popover-body")).geometry!;
    const other = (await mustFind("detach-button")).geometry!;
    if (!encloses(trigger.geometry, panel)) {
      throw new Error(`panel origin ${panel.x},${panel.y} is not on the trigger ${JSON.stringify(trigger.geometry)}`);
    }
    if (encloses(other, panel)) throw new Error(`panel origin ${panel.x},${panel.y} landed on detach-button, not the anchor`);
    console.log(`ND_POPOVER_ANCHOR_REF_OK a pooled popover placed at ${panel.x},${panel.y}, inside the node anchorRef names (ref=${trigger.ref})`);
  } else {
    console.log(`ND_POPOVER_ANCHOR_REF_OK a pooled popover presents against the node anchorRef names (ref=${trigger.ref})`);
  }

  // ---- leg 2: dropping the ref leaves it nothing to anchor on --------------
  if (gtk) {
    // Dropping the anchor unparents the popover, and GTK closes it on the way
    // out, so the app's own `open` goes false without any keystroke.
    await app.getByTestId("detach-button").click();
    await app.waitForValue("mode-label", "detached", { timeoutMs: T });
    await app.waitForValue("open-label", "shut", { timeoutMs: T });
  } else {
    await app.getByTestId("anchor-button").press("Escape");
    await app.waitForValue("open-label", "shut", { timeoutMs: T });
    await poll(() => shown(), (v) => !v, { timeoutMs: T });
    await app.getByTestId("detach-button").click();
    await app.waitForValue("mode-label", "detached", { timeoutMs: T });
  }
  await poll(() => shown(), (v) => !v, { timeoutMs: T });
  await app.getByTestId("anchor-button").click();
  await app.waitForValue("open-label", "open", { timeoutMs: T });
  await new Promise((r) => setTimeout(r, 800));
  if (await shown()) throw new Error("the popover still presented after the anchor ref was dropped: the removal never reset it");
  console.log("ND_POPOVER_ANCHOR_DROP_OK dropping the ref resets the anchor, and a pooled popover has no tree parent to fall back to");

  console.log("ND_POPOVER_ANCHOR_OK");
} finally {
  await app.close();
}
