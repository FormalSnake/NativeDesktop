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
//   --restart:       click the overlay's own Restart button (a host-drawn node,
//                    not part of the app tree), then assert the respawned app
//                    is back and the overlay's nodes have left the tree ->
//                    ERRORS_RESTART_OK. Dev mode only (the button is dev-gated).
import { connectApp, expect } from "@nativedesktop/test";

const mode = process.argv.includes("--survive")
  ? "survive"
  : process.argv.includes("--fatal")
    ? "fatal"
    : process.argv.includes("--restart")
      ? "restart"
      : "overlay-check";

const app = await connectApp();

if (mode === "survive") {
  const tree = await app.tree();
  if (tree.coordinateSpace !== "logical-window-topleft") throw new Error("bad coordinate space");

  await app.getByTestId("reject-async").click();
  await app.getByTestId("bump").click();
  await app.waitForText("Count: 1", { timeoutMs: 3000 });

  await app.getByTestId("throw-caught").click();
  await app.waitForText("caught: render-throw", { timeoutMs: 3000 });

  const fallback = app.getByTestId("boundary-fallback");
  await expect(fallback).toContainText("render-throw");
  const fallbackText = await fallback.textContent();

  console.log(`ERRORS_SURVIVE_OK fallback=${JSON.stringify(fallbackText)}`);
  app.close();
  process.exit(0);
}

if (mode === "fatal") {
  await app.getByTestId("throw-sync").click();
  console.log("ERRORS_FATAL_CLICKED");
  app.close();
  process.exit(0);
}

if (mode === "restart") {
  await app.getByTestId("nd-overlay-restart").click();
  // The respawned child re-renders the app tree, and clearing the overlay drops
  // every overlay-generation node the host registered.
  await app.waitForPresent("bump", { timeoutMs: 15000 });
  await app.waitForGone("nd-overlay-error", { timeoutMs: 15000 });
  console.log("ERRORS_RESTART_OK");
  app.close();
  process.exit(0);
}

// overlay-check
const errNode = app.getByTestId("nd-overlay-error");
await expect(errNode).toContainText("sync-throw");
await expect(errNode).not.toContainText("async-reject");
const errText = await errNode.textContent();
console.log(`ERRORS_OVERLAY_OK error=${JSON.stringify(errText)}`);
app.close();
