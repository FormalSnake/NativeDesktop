// §1.5's ND_AUTOMATION_DIALOG_SCRIPT: typed builder for the per-method FIFO
// consumed by src/automation_dialogs.zig. Entry shapes mirror the real
// systemRequest/widgetCommand results verbatim (verified against
// packages/react/src/system.ts's `dialog` object and
// packages/react/src/dialogs.ts's Window-scoped helpers) — NOT the
// {paths:[...]}/{button:0} shapes sketched in the design doc, which predate
// the landed result types:
//   - dialog.openFile resolves to a raw string[] (system.ts: `Promise<string[]>`)
//   - dialog.saveFile resolves to a raw string | null
//   - dialog.showMessage resolves to a raw number (button index)
//   - window.showAlert / .openFile / .saveFile settle dialogs.ts's
//     AlertResult / OpenFileResult / SaveFileResult objects
export interface DialogScript {
  "dialog.openFile"?: string[][];
  "dialog.saveFile"?: (string | null)[];
  "dialog.showMessage"?: number[];
  "window.showAlert"?: { buttonId: string }[];
  "window.openFile"?: { canceled: boolean; paths: string[] }[];
  "window.saveFile"?: { canceled: boolean; path: string | null }[];
}

/** Serializes a DialogScript to the inline-JSON form of ND_AUTOMATION_DIALOG_SCRIPT. */
export function dialogScriptEnv(script: DialogScript): string {
  return JSON.stringify(script);
}
