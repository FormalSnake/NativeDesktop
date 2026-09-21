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
const pendingExtensions = new Map<string, Pending<InstalledExtension[]>>();
const pendingActions = new Map<string, Pending<ExtensionAction[]>>();
const pendingWatches = new Map<string, Pending<string[]>>();
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

export interface InstalledExtension {
  id: string;
  name: string;
  version: string;
  enabled: boolean;
  /** A `data:` URI, ready for an `<image>` src. Empty when the extension has no icon. */
  iconUrl: string;
  /** `chrome-extension://…` options page, or "" when the extension declares none. */
  optionsUrl: string;
}

/// Lists the Chrome extensions this profile has installed. Chromium exposes its
/// extension registry to `chrome://extensions` and nowhere else, so `node` has
/// to be a `<webview>` showing that page (a hidden one is the usual shape), on
/// the Chromium engine with `webview.cef.style: "chrome"`. Requires the
/// `onExtensionsList` prop. An extension's action popup and options page open
/// by pointing another `<webview>` at the URL: create it with that `url`, since
/// Chromium refuses a renderer-initiated navigation to a `chrome-extension://`
/// page.
export function listExtensions(node: NdNodeRef<"webview">): Promise<InstalledExtension[]> {
  const id = nextId("ext");
  return new Promise<InstalledExtension[]>((resolve, reject) => {
    pendingExtensions.set(id, { resolve, reject });
    sendCommand(node, "listExtensions", { id });
  });
}

/// Pass as a <webview>'s `onExtensionsList` prop, which settles the
/// listExtensions() call whose `id` matches this event's payload.
export function onExtensionsList(e: { data: unknown }): void {
  const result = e.data as { id: string; ok: boolean; extensions?: InstalledExtension[]; error?: string };
  const call = pendingExtensions.get(result.id);
  if (!call) return;
  pendingExtensions.delete(result.id);
  if (result.ok) call.resolve(result.extensions ?? []);
  else call.reject(new Error(result.error ?? "listExtensions failed"));
}

/// Loads an unpacked extension from a local directory into the live profile,
/// with no relaunch and no `--load-extension`. `node` is the same
/// `chrome://extensions` view `listExtensions` uses; the answer is the registry
/// as it now stands, so it settles through `onExtensionsList`.
export function installExtension(node: NdNodeRef<"webview">, path: string): Promise<InstalledExtension[]> {
  return extensionMutation(node, "installExtension", { path });
}

export function uninstallExtension(node: NdNodeRef<"webview">, extensionId: string): Promise<InstalledExtension[]> {
  return extensionMutation(node, "uninstallExtension", { extensionId });
}

export function setExtensionEnabled(
  node: NdNodeRef<"webview">,
  extensionId: string,
  enabled: boolean,
): Promise<InstalledExtension[]> {
  return extensionMutation(node, "setExtensionEnabled", { extensionId, enabled });
}

function extensionMutation(
  node: NdNodeRef<"webview">,
  command: string,
  args: Record<string, unknown>,
): Promise<InstalledExtension[]> {
  const id = nextId("ext");
  return new Promise<InstalledExtension[]>((resolve, reject) => {
    pendingExtensions.set(id, { resolve, reject });
    sendCommand(node, command, { id, ...args });
  });
}

/// Why the registry changed, as Chromium spelled it: `developerPrivate`'s own
/// vocabulary (`INSTALLED`, `UNINSTALLED`, `LOADED`, `UNLOADED`,
/// `PREFS_CHANGED`, …) or the `chrome.management` event name that fired. Treat
/// it as a hint and re-read the registry; the set is Chromium's, not this
/// framework's.
export interface ExtensionsChange {
  reason: string;
}

/// One list, not one per view: an event carries the payload and nothing that
/// names the view it came from, and Chromium exposes its registry to one page,
/// so an app has one of these views.
const extensionsWatchers: Array<(change: ExtensionsChange) => void> = [];

/// Subscribes to the registry's own change events, so an app learns that an
/// extension was installed, removed, enabled, disabled or updated instead of
/// polling for it. A Web Store install happens entirely inside Chromium and
/// reaches no other `<webview>` callback.
///
/// `node` is the same `chrome://extensions` view `listExtensions` uses, and the
/// subscription belongs to that document: call it again after the view
/// reloads. Resolves with the event sources it attached to, which is what the
/// page really exposes rather than what this framework hoped for. `listener`
/// runs on every later change.
export function watchExtensions(
  node: NdNodeRef<"webview">,
  listener: (change: ExtensionsChange) => void,
): Promise<string[]> {
  const id = nextId("ext");
  return new Promise<string[]>((resolve, reject) => {
    pendingWatches.set(id, { resolve, reject });
    extensionsWatchers.push(listener);
    sendCommand(node, "watchExtensions", { id });
  });
}

/// Pass as a <webview>'s `onExtensionsChanged` prop. It settles the
/// watchExtensions() call that carries the same `id`, and delivers every later
/// change to the listeners registered with it.
export function onExtensionsChanged(e: { data: unknown }): void {
  const result = e.data as { id?: string; ok?: boolean; sources?: string[]; error?: string; reason?: string };
  if (result.id !== undefined) {
    const call = pendingWatches.get(result.id);
    if (!call) return;
    pendingWatches.delete(result.id);
    if (result.ok) call.resolve(result.sources ?? []);
    else call.reject(new Error(result.error ?? "watchExtensions failed"));
    return;
  }
  for (const listener of extensionsWatchers) listener({ reason: result.reason ?? "" });
}

export interface ExtensionAction {
  id: string;
  name: string;
  enabled: boolean;
  /** `action.default_title`, falling back to the extension's name. */
  title: string;
  /** `chrome-extension://…` icon from `action.default_icon`, or the registry icon. */
  iconUrl: string;
  /** `chrome-extension://…` popup page, or "" for an action that fires `onClicked`. */
  popupUrl: string;
  /** Always "": Chromium answers `chrome.action.getBadgeText` to the extension alone. */
  badgeText: string;
}

/// The actions the installed extensions declare, for an app that draws its own
/// toolbar. Same view as `listExtensions`; requires the `onExtensionActions`
/// prop. Clicking one means mounting a `<webview>` created at `popupUrl`: there
/// is no toolbar button for Chromium to consider clicked, so `onClicked` never
/// fires and no `activeTab` grant is issued.
export function listExtensionActions(node: NdNodeRef<"webview">): Promise<ExtensionAction[]> {
  const id = nextId("ext");
  return new Promise<ExtensionAction[]>((resolve, reject) => {
    pendingActions.set(id, { resolve, reject });
    sendCommand(node, "listExtensionActions", { id });
  });
}

/// Pass as a <webview>'s `onExtensionActions` prop.
export function onExtensionActions(e: { data: unknown }): void {
  const result = e.data as { id: string; ok: boolean; actions?: ExtensionAction[]; error?: string };
  const call = pendingActions.get(result.id);
  if (!call) return;
  pendingActions.delete(result.id);
  if (result.ok) call.resolve(result.actions ?? []);
  else call.reject(new Error(result.error ?? "listExtensionActions failed"));
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
