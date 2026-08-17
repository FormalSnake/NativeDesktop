// Promise-correlating helpers for the <webview> commands whose answer comes
// back as an event rather than a return value. The host runs the work
// out-of-band and replies async over NDP with the matching `id`, so these
// modules hold the resolver until that event arrives.

import type { NdNodeRef } from "./generated/intrinsics.ts";
import { sendCommand } from "./renderer.ts";

interface Pending<T> {
  resolve: (value: T) => void;
  reject: (reason: Error) => void;
}

const pendingEvals = new Map<string, Pending<string>>();
const pendingCookies = new Map<string, Pending<Cookie[]>>();
const pendingSessions = new Map<string, Pending<string>>();
let seq = 0;

function nextId(prefix: string): string {
  return `${prefix}${++seq}`;
}

/// Runs `code` in the given <webview>'s page. Resolves with the host's
/// serialized result once the matching `javaScriptResult` event arrives;
/// rejects with the host-reported error when `ok` is false. `world` names an
/// isolated JavaScript world (the same names `addUserScript` uses); omit it to
/// run in the page's own world.
export function executeJavaScript(node: NdNodeRef<"webview">, code: string, world?: string): Promise<string> {
  const id = nextId("js");
  return new Promise<string>((resolve, reject) => {
    pendingEvals.set(id, { resolve, reject });
    sendCommand(node, "executeJavaScript", world ? { id, code, world } : { id, code });
  });
}

/// Pass as a <webview>'s `onJavaScriptResult` prop — resolves or rejects the
/// executeJavaScript() call whose `id` matches this event's payload.
export function onJavaScriptResult(e: { data: unknown }): void {
  const result = e.data as { id: string; ok: boolean; value?: string; error?: string };
  const call = pendingEvals.get(result.id);
  if (!call) return;
  pendingEvals.delete(result.id);
  if (result.ok) call.resolve(result.value ?? "");
  else call.reject(new Error(result.error ?? "executeJavaScript failed"));
}

export interface Cookie {
  name: string;
  value: string;
  domain: string;
  path: string;
  secure: boolean;
  httpOnly: boolean;
  /** Unix seconds, or null for a session cookie. */
  expires: number | null;
  sameSite: "None" | "Lax" | "Strict";
}

/// Reads the cookies visible to this <webview>'s profile. With `url`, only the
/// cookies that apply to that URL's host. Requires the `onCookiesResult` prop.
export function getCookies(node: NdNodeRef<"webview">, url?: string): Promise<Cookie[]> {
  const id = nextId("ck");
  return new Promise<Cookie[]>((resolve, reject) => {
    pendingCookies.set(id, { resolve, reject });
    sendCommand(node, "getCookies", url ? { id, url } : { id });
  });
}

/// Pass as a <webview>'s `onCookiesResult` prop — settles the getCookies()
/// call whose `id` matches this event's payload.
export function onCookiesResult(e: { data: unknown }): void {
  const result = e.data as { id: string; ok: boolean; cookies?: Cookie[]; error?: string };
  const call = pendingCookies.get(result.id);
  if (!call) return;
  pendingCookies.delete(result.id);
  if (result.ok) call.resolve(result.cookies ?? []);
  else call.reject(new Error(result.error ?? "getCookies failed"));
}

/// Captures the view's navigation history as an opaque base64 blob, restorable
/// with `sendCommand(node, "restoreSession", { state })`. Requires the
/// `onSessionSaved` prop.
export function saveSession(node: NdNodeRef<"webview">): Promise<string> {
  const id = nextId("ss");
  return new Promise<string>((resolve, reject) => {
    pendingSessions.set(id, { resolve, reject });
    sendCommand(node, "saveSession", { id });
  });
}

/// Pass as a <webview>'s `onSessionSaved` prop — resolves the saveSession()
/// call whose `id` matches this event's payload.
export function onSessionSaved(e: { data: unknown }): void {
  const result = e.data as { id: string; state: string };
  const call = pendingSessions.get(result.id);
  if (!call) return;
  pendingSessions.delete(result.id);
  call.resolve(result.state ?? "");
}

/// Where an item may appear. `page` means the click landed on nothing more
/// specific: a link, image, selection or editable field wins over it, which is
/// what a browser's own menu does.
export type ContextMenuContext = "all" | "page" | "link" | "image" | "selection" | "editable";

export interface ContextMenuItem {
  /// Echoed back as `id` on `contextMenuItemClicked`. Omitted for a separator.
  id?: string;
  label?: string;
  type?: "normal" | "checkbox" | "radio" | "separator";
  /// Drawn as the item's state. The framework never mutates it: a click reports
  /// the state it implies and the app answers with the next
  /// `setContextMenuItems`.
  checked?: boolean;
  enabled?: boolean;
  /// Defaults to `["page"]`.
  contexts?: ContextMenuContext[];
  /// `*`-wildcard globs matched against the link or image URL under the
  /// pointer. An item with globs and no matching target is not shown.
  targetUrlGlobs?: string[];
  /// A submenu. Not depth-limited, but two levels is what menus stay readable
  /// at.
  children?: ContextMenuItem[];
}

export interface ContextMenuItemClick {
  id: string;
  pageUrl: string;
  linkUrl?: string;
  imageUrl?: string;
  /// macOS only: WebKitGTK's hit test reports THAT there is a selection
  /// without its text.
  selectionText?: string;
  editable: boolean;
  /// Checkbox and radio items only: the state the click implies, and the one it
  /// replaced.
  checked?: boolean;
  wasChecked?: boolean;
}

/// Replaces the items this <webview> merges into the engine's own context menu.
/// Only meaningful with `contextMenuMode="native"` (the default): in
/// `"suppress"` mode no engine menu opens, so there is nothing to merge into.
/// Clicks arrive on the `onContextMenuItemClicked` prop.
export function setContextMenuItems(node: NdNodeRef<"webview">, items: ContextMenuItem[]): void {
  sendCommand(node, "setContextMenuItems", { items });
}
