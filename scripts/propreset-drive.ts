#!/usr/bin/env bun
// scripts/propreset-drive.ts [gtk|appkit] -- drives examples/propreset/main.tsx
// via @nativedesktop/test. Acceptance for dropped-prop resets: a prop that
// leaves the app's JSX reaches the host as null and the generated appliers
// substitute the schema default, so the native widget resets instead of
// keeping its last value. Prints ND_PROP_RESET_OK on success.
//
// Runs either way round: scripts/headless-popover.sh's sibling
// scripts/headless-propreset.sh owns the host and hands the socket over in
// ND_AUTOMATION_SOCKET, and a bare `bun scripts/propreset-drive.ts` launches
// one itself. Every leg is backend-neutral, so nothing here branches on
// ND_BACKEND.
import { connectApp, findNode, launchApp } from "../packages/test/src/index.ts";
import type { Backend } from "@nativedesktop/host";

const backend = process.argv[2] as Backend | undefined;
const attached = process.env.ND_AUTOMATION_SOCKET != null;

const app = attached ? await connectApp() : await launchApp({ entry: "examples/propreset/main.tsx", backend });
const mustFind = async (testId: string) => {
  const node = findNode((await app.tree()).root, testId);
  if (!node) throw new Error(`${testId} not found in tree`);
  return node;
};
try {
  // ---- leg 1: the props are set, so the widgets carry them ------------------
  const before = await mustFind("subject-button");
  if (before.text !== "Subject") throw new Error(`button text before=${JSON.stringify(before.text)}, want "Subject"`);
  if (before.enabled !== false) throw new Error(`button enabled before=${before.enabled}, want false`);
  if (before.label !== "Subject hint") throw new Error(`button label before=${JSON.stringify(before.label)}, want "Subject hint"`);
  await app.waitForValue("subject-check", true, { timeoutMs: 3000 });
  console.log("ND_PROP_RESET_SET_OK label/tooltip/enabled/checked all applied");

  // ---- leg 2: one click drops every prop from the render --------------------
  await app.getByTestId("drop-toggle").click();
  await app.waitForText("dropped", { timeoutMs: 3000 });

  // `enabled` defaults to true, `checked` to false, `label`/`tooltip` have no
  // schema default so they reset to the empty string.
  await app.waitFor({ testId: "subject-button", state: "enabled" }, { timeoutMs: 3000 });
  await app.waitForValue("subject-check", false, { timeoutMs: 3000 });
  const after = await mustFind("subject-button");
  if (after.text !== "") throw new Error(`button text after=${JSON.stringify(after.text)}, want ""`);
  if (after.label != null) throw new Error(`button label after=${JSON.stringify(after.label)}, want none`);
  const check = await mustFind("subject-check");
  if (check.text !== "") throw new Error(`checkbox text after=${JSON.stringify(check.text)}, want ""`);
  console.log("ND_PROP_RESET_DROP_OK enabled->true checked->false label/tooltip->empty");

  // ---- leg 3: a dropped `style` unstyles the box ----------------------------
  // Nothing in getTree reports style keys, so the padding shows up as the box
  // shrinking back onto its one child.
  const box = await mustFind("subject-box");
  const label = await mustFind("styled-label");
  const pad = (box.geometry?.h ?? 0) - (label.geometry?.h ?? 0);
  if (pad > 8) throw new Error(`box still padded after style drop: box h=${box.geometry?.h} label h=${label.geometry?.h}`);
  console.log(`ND_PROP_RESET_STYLE_OK box collapsed onto its child (slack=${pad}px)`);

  console.log("ND_PROP_RESET_OK");
} finally {
  await app.close();
}
