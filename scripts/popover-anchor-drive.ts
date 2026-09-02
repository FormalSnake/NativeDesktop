#!/usr/bin/env bun
// scripts/popover-anchor-drive.ts [gtk|appkit] -- drives
// examples/popoveranchor/main.tsx via @nativedesktop/test. Acceptance for
// <popover anchorRef>: the popover is rendered through a portal into the
// off-window pool, so it has no tree parent to anchor on and `anchorRef` is
// the only thing that can place it. A popover's content lives in its own
// window, whose coordinates getTree reports in that window's own space, so
// placement is asserted as the thing a user would notice: with the ref the
// panel presents (its content becomes visible), and with the ref dropped it
// has nowhere to open. Prints ND_POPOVER_ANCHOR_OK on success.
import { launchApp, poll, findNode } from "../packages/test/src/index.ts";
import type { Backend } from "@nativedesktop/host";

const backend = process.argv[2] as Backend | undefined;
const T = 4000;

const app = await launchApp({ entry: "examples/popoveranchor/main.tsx", backend });
const shown = async () => findNode((await app.tree()).root, "popover-body")?.visible === true;
try {
  const trigger = await app.mustFind("anchor-button");
  if (!trigger.geometry?.w) throw new Error("the trigger button never laid out");
  if (await shown()) throw new Error("the popover was already presenting before anything opened it");

  // ---- leg 1: the ref places a popover with no tree parent at all ----------
  await app.getByTestId("anchor-button").click();
  await app.waitForValue("open-label", "open", { timeoutMs: T });
  await poll(() => shown(), (v) => v, { timeoutMs: T });
  console.log(`ND_POPOVER_ANCHOR_REF_OK a pooled popover presents against the node anchorRef names (ref=${trigger.ref})`);

  // ---- leg 2: dropping the ref leaves it nothing to anchor on --------------
  await app.getByTestId("anchor-button").press("Escape");
  await app.waitForValue("open-label", "shut", { timeoutMs: T });
  await poll(() => shown(), (v) => !v, { timeoutMs: T });
  await app.getByTestId("detach-button").click();
  await app.waitForValue("mode-label", "detached", { timeoutMs: T });
  await app.getByTestId("anchor-button").click();
  await app.waitForValue("open-label", "open", { timeoutMs: T });
  await new Promise((r) => setTimeout(r, 800));
  if (await shown()) throw new Error("the popover still presented after the anchor ref was dropped: the removal never reset it");
  console.log("ND_POPOVER_ANCHOR_DROP_OK dropping the ref resets the anchor, and a pooled popover has no tree parent to fall back to");

  console.log("ND_POPOVER_ANCHOR_OK");
} finally {
  await app.close();
}
