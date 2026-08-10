#!/usr/bin/env bun
// scripts/dialog-script-drive.ts [gtk|appkit] — exercises §1.5's
// ND_AUTOMATION_DIALOG_SCRIPT against examples/dialogs: both interception
// paths (app-level dialog.* via systemRequest, window-scoped showAlert/
// openFile/saveFile via widgetCommand), and the exhausted-queue contract
// (loud stderr diagnostic, never a silently-shown real dialog).
//
// examples/gallery wires the same six calls behind a TabView page, which
// automation can't reach yet (no page-switch RPC — automation-socket.md's
// "Known gaps"); examples/dialogs puts all six triggers directly on the
// window so this script needs no tab-switching story. See that example's
// header comment for the full rationale.
import { launchApp } from "../packages/test/src/index.ts";
import type { DialogScript } from "../packages/test/src/index.ts";
import type { Backend } from "@nativedesktop/host";

const backend = process.argv[2] as Backend | undefined;

const script: DialogScript = {
  "dialog.openFile": [["/tmp/nd-dialog-script-a.txt"]],
  "dialog.saveFile": ["/tmp/nd-dialog-script-out.txt"],
  "dialog.showMessage": [1],
  "window.showAlert": [{ buttonId: "delete" }],
  "window.openFile": [{ canceled: false, paths: ["/tmp/nd-dialog-script-b.txt"] }],
  "window.saveFile": [{ canceled: true, path: null }],
};

const app = await launchApp({ entry: "examples/dialogs/main.tsx", backend, dialogScript: script });

// Result labels render as `Result: ${value}` (examples/dialogs/main.tsx) —
// waitForText searches the whole tree's `text` field, not the a11y `value`
// probe (which is null for a Label), so match on that literal prefix.
async function clickAndExpect(button: string, resultTestId: string, expectedValue: string): Promise<void> {
  await app.click(button);
  const expectedText = `Result: ${expectedValue}`;
  const result = await app.waitForText(expectedText, { timeoutMs: 3000 });
  if (!result.matched) {
    const node = await app.find(resultTestId);
    throw new Error(`${button}: never saw "${expectedText}" (last saw ${JSON.stringify(node?.text)})`);
  }
}

// App-level (systemRequest, packages/react/src/system.ts's `dialog` object):
// raw result shapes, not wrapped objects — see src/dialogs.ts's header comment.
await clickAndExpect("app-open-file-button", "app-open-file-result", "/tmp/nd-dialog-script-a.txt");
await clickAndExpect("app-save-file-button", "app-save-file-result", "/tmp/nd-dialog-script-out.txt");
await clickAndExpect("app-show-message-button", "app-show-message-result", "1");
console.log("ND_DIALOG_SCRIPT_APP_OK dialog.openFile/saveFile/showMessage all scripted");

// Window-scoped (widgetCommand, packages/react/src/dialogs.ts): the real
// AlertResult/OpenFileResult/SaveFileResult shapes.
await clickAndExpect("window-show-alert-button", "window-alert-result", "delete");
await clickAndExpect("window-open-file-button", "window-open-file-result", "/tmp/nd-dialog-script-b.txt");
await clickAndExpect("window-save-file-button", "window-save-file-result", "canceled");
console.log("ND_DIALOG_SCRIPT_WINDOW_OK window.showAlert/openFile/saveFile all scripted");

// Exhausted queue: each method's FIFO above has exactly one entry. A second
// click must never fall through to a real (blocking, headless-hanging)
// dialog — the host answers loudly on stderr instead.
await app.click("app-open-file-button");
await app.waitForMarker("ND_DIALOG_SCRIPT_EXHAUSTED method=dialog.openFile", 3000);
await app.click("window-show-alert-button");
await app.waitForMarker("ND_DIALOG_SCRIPT_EXHAUSTED method=window.showAlert", 3000);
console.log("ND_DIALOG_SCRIPT_EXHAUSTED_OK a drained queue fails loudly on both interception paths, never shows real UI");

await app.close();
console.log(`ND_DIALOG_SCRIPT_OK backend=${app.backend} openFile/saveFile/showAlert scripted on both paths, exhausted queue verified`);
