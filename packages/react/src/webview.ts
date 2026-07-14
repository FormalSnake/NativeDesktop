// Promise-correlating helper for <webview>'s `executeJavaScript` command.
// The host runs `code` in the page's JS context out-of-band and replies
// async over NDP with a `javaScriptResult` event carrying the same `id` —
// there is no synchronous return path, so this module holds the resolver
// until that event comes back.

import type { NdNodeRef } from "./generated/intrinsics.ts";
import { sendCommand } from "./renderer.ts";

interface PendingEval {
  resolve: (value: string) => void;
  reject: (reason: Error) => void;
}

const pending = new Map<string, PendingEval>();
let seq = 0;

/// Runs `code` in the given <webview>'s page. Resolves with the host's
/// serialized result once the matching `javaScriptResult` event arrives;
/// rejects with the host-reported error when `ok` is false.
export function executeJavaScript(node: NdNodeRef<"webview">, code: string): Promise<string> {
  const id = `js${++seq}`;
  return new Promise<string>((resolve, reject) => {
    pending.set(id, { resolve, reject });
    sendCommand(node, "executeJavaScript", { id, code });
  });
}

/// Pass as a <webview>'s `onJavaScriptResult` prop — resolves or rejects the
/// executeJavaScript() call whose `id` matches this event's payload.
export function onJavaScriptResult(e: { data: unknown }): void {
  const result = e.data as { id: string; ok: boolean; value?: string; error?: string };
  const call = pending.get(result.id);
  if (!call) return;
  pending.delete(result.id);
  if (result.ok) call.resolve(result.value ?? "");
  else call.reject(new Error(result.error ?? "executeJavaScript failed"));
}
