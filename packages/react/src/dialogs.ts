// Promise-correlating helpers for the <window> widget's native dialog
// commands (showAlert/openFile/saveFile/showAbout). Same shape as
// webview.ts's executeJavaScript: sendCommand() kicks the native dialog off,
// and the matching *Result event that the host fires back settles the
// promise this module is holding.
//
// Unlike executeJavaScript, the schema gives these commands no caller-
// supplied `id` to correlate by — a modal dialog is one-per-window on both
// backends (NSAlert/NSOpenPanel/NSSavePanel are window-attached sheets;
// AdwAlertDialog/GtkFileDialog are the same), so there's nothing to
// disambiguate beyond "which window". Correlation therefore keys off the
// window node's own wire id instead of a generated token, and every helper
// here — including the result handlers — takes that node explicitly so
// multiple windows never share state. Wire a window's result props as:
//
//   const winRef = useRef<NdNodeRef<"window">>(null);
//   <window ref={winRef}
//     onAlertResult={(e) => onAlertResult(winRef.current!, e)}
//     onOpenFileResult={(e) => onOpenFileResult(winRef.current!, e)}
//     onSaveFileResult={(e) => onSaveFileResult(winRef.current!, e)}
//   />
//
// Only one dialog may be pending per window at a time — the host has no
// queueing story for two native sheets stacked on one window, so a second
// call while one is outstanding REJECTS immediately rather than silently
// queueing or clobbering the first caller's promise.

import type { NdNodeRef } from "./generated/intrinsics.ts";
import { sendCommand } from "./renderer.ts";
import type { FileFilter } from "./system.ts";

export type { FileFilter };

export interface DialogButton {
  id: string;
  label: string;
  style?: "default" | "suggested" | "destructive";
}

export interface ShowAlertOptions {
  title: string;
  body?: string;
  buttons: DialogButton[];
}

export interface AlertResult {
  buttonId: string;
}

export interface OpenFileOptions {
  multiple?: boolean;
  directories?: boolean;
  filters?: FileFilter[];
}

export interface OpenFileResult {
  canceled: boolean;
  paths: string[];
}

export interface SaveFileOptions {
  suggestedName?: string;
  defaultDir?: string;
  filters?: FileFilter[];
}

export interface SaveFileResult {
  canceled: boolean;
  path: string | null;
}

export interface ShowAboutOptions {
  appName: string;
  version: string;
  developer?: string;
  website?: string;
}

type DialogKind = "showAlert" | "openFile" | "saveFile";

type PendingDialog =
  | { kind: "showAlert"; resolve: (value: AlertResult) => void; reject: (reason: Error) => void }
  | { kind: "openFile"; resolve: (value: OpenFileResult) => void; reject: (reason: Error) => void }
  | { kind: "saveFile"; resolve: (value: SaveFileResult) => void; reject: (reason: Error) => void };

const pending = new Map<number, PendingDialog>();

/// Rejects `reject` and returns false if `node` already has a dialog
/// pending; otherwise returns true (caller still owes a `pending.set(...)`).
function checkAvailable(node: NdNodeRef<"window">, reject: (reason: Error) => void): boolean {
  const existing = pending.get(node.id);
  if (!existing) return true;
  reject(
    new Error(`<window> already has a "${existing.kind}" dialog pending; only one modal dialog per window is allowed at a time`),
  );
  return false;
}

function settle(node: NdNodeRef<"window">, kind: DialogKind, data: unknown): void {
  const call = pending.get(node.id);
  if (!call || call.kind !== kind) return; // stray/mismatched event — ignore
  pending.delete(node.id);
  switch (call.kind) {
    case "showAlert":
      call.resolve(data as AlertResult);
      break;
    case "openFile":
      call.resolve(data as OpenFileResult);
      break;
    case "saveFile":
      call.resolve(data as SaveFileResult);
      break;
  }
}

/// Shows a native modal alert sheet on `node`. Resolves with the id of the
/// button the user clicked.
export function showAlert(node: NdNodeRef<"window">, options: ShowAlertOptions): Promise<AlertResult> {
  return new Promise<AlertResult>((resolve, reject) => {
    if (!checkAvailable(node, reject)) return;
    pending.set(node.id, { kind: "showAlert", resolve, reject });
    sendCommand(node, "showAlert", options);
  });
}

/// Opens a native "choose file(s)" dialog scoped to `node`'s window.
/// Resolves `{ canceled: true, paths: [] }` if the user cancels.
export function openFile(node: NdNodeRef<"window">, options: OpenFileOptions = {}): Promise<OpenFileResult> {
  return new Promise<OpenFileResult>((resolve, reject) => {
    if (!checkAvailable(node, reject)) return;
    pending.set(node.id, { kind: "openFile", resolve, reject });
    sendCommand(node, "openFile", options);
  });
}

/// Opens a native "save file" dialog scoped to `node`'s window. Resolves
/// `{ canceled: true, path: null }` if the user cancels.
export function saveFile(node: NdNodeRef<"window">, options: SaveFileOptions = {}): Promise<SaveFileResult> {
  return new Promise<SaveFileResult>((resolve, reject) => {
    if (!checkAvailable(node, reject)) return;
    pending.set(node.id, { kind: "saveFile", resolve, reject });
    sendCommand(node, "saveFile", options);
  });
}

/// Shows the app's native About panel. No result event (mirrors webview's
/// goBack/reload/stop) — fire-and-forget, and unlike the other three this
/// does NOT claim the per-window dialog slot, since there is no pending
/// promise here for a concurrent call to clobber.
export function showAbout(node: NdNodeRef<"window">, options: ShowAboutOptions): void {
  sendCommand(node, "showAbout", options);
}

/// Opens the native tab overview for the window's tab group — AdwTabOverview
/// on GNOME, Show All Tabs (toggleTabOverview) on macOS. Fire-and-forget like
/// showAbout; a no-op on windows without a `tabGroup`.
export function showTabOverview(node: NdNodeRef<"window">): void {
  sendCommand(node, "showTabOverview", {});
}

/// Pass as the owning <window>'s `onAlertResult` prop, bound to that same
/// node (see module header) — settles the matching showAlert() call.
export function onAlertResult(node: NdNodeRef<"window">, e: { data: unknown }): void {
  settle(node, "showAlert", e.data);
}

/// Pass as the owning <window>'s `onOpenFileResult` prop, bound to that same
/// node — settles the matching openFile() call.
export function onOpenFileResult(node: NdNodeRef<"window">, e: { data: unknown }): void {
  settle(node, "openFile", e.data);
}

/// Pass as the owning <window>'s `onSaveFileResult` prop, bound to that same
/// node — settles the matching saveFile() call.
export function onSaveFileResult(node: NdNodeRef<"window">, e: { data: unknown }): void {
  settle(node, "saveFile", e.data);
}
