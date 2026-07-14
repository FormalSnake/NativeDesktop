// Promise-correlating helper for <toastoverlay>'s `showToast` command.
// Mirrors webview.ts's executeJavaScript pattern exactly: this module
// generates the correlation `id`, sends it as part of the showToast command
// arg, and holds the resolver until a matching event echoes that same id
// back over NDP.
//
// Unlike executeJavaScript, two DIFFERENT events can settle the same pending
// call — a click resolves `{ buttonClicked: true }`, any other dismissal
// (timeout, ESC, or AdwToastOverlay's queue advancing past it) resolves
// `{ buttonClicked: false }` — so `pending` is cleared by whichever fires
// first and the other is a no-op.

import type { NdNodeRef } from "./generated/intrinsics.ts";
import { sendCommand } from "./renderer.ts";

export type ToastPriority = "normal" | "high";

export interface ShowToastOptions {
  title: string;
  buttonLabel?: string;
  timeoutSeconds?: number;
  priority?: ToastPriority;
}

export interface ToastResult {
  buttonClicked: boolean;
}

interface PendingToast {
  resolve: (value: ToastResult) => void;
}

const pending = new Map<string, PendingToast>();
let seq = 0;

/// Queues a toast on the given <toastoverlay>. Resolves `{ buttonClicked:
/// true }` if the user clicks the toast's action button, `{ buttonClicked:
/// false }` if it's dismissed any other way.
export function showToast(node: NdNodeRef<"toastoverlay">, options: ShowToastOptions): Promise<ToastResult> {
  const id = `toast${++seq}`;
  return new Promise<ToastResult>((resolve) => {
    pending.set(id, { resolve });
    sendCommand(node, "showToast", { id, ...options });
  });
}

/// Dismisses a toast on the given <toastoverlay> — the one matching `id` if
/// given, otherwise whichever toast is currently visible.
export function dismissToast(node: NdNodeRef<"toastoverlay">, id?: string): void {
  sendCommand(node, "dismissToast", id ? { id } : {});
}

/// Pass as a <toastoverlay>'s `onToastButtonClicked` prop — settles the
/// matching showToast() call with `{ buttonClicked: true }`.
export function onToastButtonClicked(e: { data: unknown }): void {
  const { id } = e.data as { id: string };
  const toast = pending.get(id);
  if (!toast) return;
  pending.delete(id);
  toast.resolve({ buttonClicked: true });
}

/// Pass as a <toastoverlay>'s `onToastDismissed` prop — settles the matching
/// showToast() call (if still pending) with `{ buttonClicked: false }`.
export function onToastDismissed(e: { data: unknown }): void {
  const { id } = e.data as { id: string };
  const toast = pending.get(id);
  if (!toast) return;
  pending.delete(id);
  toast.resolve({ buttonClicked: false });
}
