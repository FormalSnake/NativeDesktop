// Promise-correlating helpers for the <webview> commands whose answer comes
// back as an event rather than a return value. The host runs the work
// out-of-band and replies async over NDP with the matching `id`, so these
// modules hold the resolver until that event arrives.

import type { NdNodeRef } from "./generated/widgets.ts";
import { getSession } from "./session.ts";
import { onNodeRemoved } from "./ops.ts";
import { sendCommand } from "./commands.ts";

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
const pendingActionState = new Map<string, Pending<ExtensionActionState>>();
const pendingTriggers = new Map<string, Pending<void>>();
let seq = 0;

function nextId(prefix: string): string {
  return `${prefix}${++seq}`;
}

/// The unanswered requests of each mounted view. A removed view answers
/// nothing, and a result it had in flight has no prop left to land on, so its
/// removal is what settles them.
const inFlight = new Map<number, Map<string, (reason: Error) => void>>();

onNodeRemoved((nodeId) => {
  const calls = inFlight.get(nodeId);
  if (!calls) return;
  inFlight.delete(nodeId);
  for (const [id, reject] of calls) reject(new Error(`the <webview> was removed before request ${id} was answered`));
});

/// Sends a command whose answer arrives as an event carrying `id`, and parks
/// its resolver in `pending` until the matching result handler settles it.
function request<T>(
  node: NdNodeRef<"webview">,
  pending: Map<string, Pending<T>>,
  prefix: string,
  command: string,
  args: Record<string, unknown>,
): Promise<T> {
  const id = nextId(prefix);
  return new Promise<T>((resolve, reject) => {
    if (!getSession()?.registry.get(node.id)) {
      reject(new Error(`${command}: the <webview> is no longer mounted`));
      return;
    }
    let calls = inFlight.get(node.id);
    if (!calls) inFlight.set(node.id, (calls = new Map()));
    const forget = (): void => {
      pending.delete(id);
      calls.delete(id);
      if (calls.size === 0 && inFlight.get(node.id) === calls) inFlight.delete(node.id);
    };
    pending.set(id, {
      resolve: (value) => {
        forget();
        resolve(value);
      },
      reject: (reason) => {
        forget();
        reject(reason);
      },
    });
    calls.set(id, (reason) => {
      forget();
      reject(reason);
    });
    try {
      sendCommand(node, command as never, { id, ...args });
    } catch (error) {
      forget();
      reject(error as Error);
    }
  });
}

/// Runs `code` in the given <webview>'s page. Resolves with the host's
/// serialized result once the matching `javaScriptResult` event arrives;
/// rejects with the host-reported error when `ok` is false. `world` names an
/// isolated JavaScript world (the same names `addUserScript` uses); omit it to
/// run in the page's own world. `userGesture` runs the code as if the user had
/// just clicked the page, for the calls a page only honours from a click
/// (`requestPictureInPicture`, `requestFullscreen`); Chromium engine only.
export function executeJavaScript(
  node: NdNodeRef<"webview">,
  code: string,
  world?: string,
  options?: { userGesture?: boolean },
): Promise<string> {
  return request(node, pendingEvals, "js", "executeJavaScript", {
    code,
    ...(world ? { world } : {}),
    ...(options?.userGesture ? { userGesture: true } : {}),
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
  return request(node, pendingCookies, "ck", "getCookies", url ? { url } : {});
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
  return request(node, pendingSessions, "ss", "saveSession", {});
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
  return request(node, pendingExtensions, "ext", "listExtensions", {});
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

/// Removes an extension without Chrome's "Remove ...?" confirmation, which has
/// no toolbar to hang from in an embedded browser: ask the person first.
/// Same view as `listExtensions`; settles with the registry once the extension
/// has left it. Chromium engine under Chrome style.
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
  return request(node, pendingExtensions, "ext", command, args);
}

/// Why the registry changed, as Chromium spelled it: `developerPrivate`'s own
/// vocabulary (`INSTALLED`, `UNINSTALLED`, `LOADED`, `UNLOADED`,
/// `PREFS_CHANGED`, …) or the `chrome.management` event name that fired. Treat
/// it as a hint and re-read the registry; the set is Chromium's, not this
/// framework's.
export interface ExtensionsChange {
  reason: string;
  /** The extension the change is about, or "" when the event does not say. */
  extensionId: string;
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
  extensionsWatchers.push(listener);
  return request(node, pendingWatches, "ext", "watchExtensions", {});
}

/// Pass as a <webview>'s `onExtensionsChanged` prop. It settles the
/// watchExtensions() call that carries the same `id`, and delivers every later
/// change to the listeners registered with it.
export function onExtensionsChanged(e: { data: unknown }): void {
  const result = e.data as { id?: string; ok?: boolean; sources?: string[]; error?: string; reason?: string; extensionId?: string };
  if (result.id !== undefined) {
    const call = pendingWatches.get(result.id);
    if (!call) return;
    pendingWatches.delete(result.id);
    if (result.ok) call.resolve(result.sources ?? []);
    else call.reject(new Error(result.error ?? "watchExtensions failed"));
    return;
  }
  for (const listener of extensionsWatchers) listener({ reason: result.reason ?? "", extensionId: result.extensionId ?? "" });
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
  /** Always "": read the live value with `readExtensionAction`. */
  badgeText: string;
}

/// An action as Chromium holds it right now, which is not what the manifest
/// says. `chrome.action.setPopup`, `setBadgeText` and `setTitle` are answered
/// to the extension alone, so this is the only honest source for them.
export interface ExtensionActionState {
  id: string;
  /// The tab the state was read for: Chromium's own active tab, which is the
  /// page the app last had focus in. 0 when Chromium had no active tab.
  tabId: number;
  tabUrl: string;
  /// The popup Chromium would open for a click. **`""` means the extension has
  /// turned its popup off**, which is a state an app must respect: opening the
  /// manifest's popup anyway shows a document the extension never meant to be
  /// on screen. 1Password clears it while no account is configured so that a
  /// click opens its onboarding instead.
  popupUrl: string;
  badgeText: string;
  /// RGBA 0..255, or [] when the extension never set one.
  badgeColor: number[];
  title: string;
  enabled: boolean;
}

/// Reads an action's live state. `node` is a `<webview>` showing any page of
/// that extension, because that is the only context Chromium tells: the WebUI
/// at `chrome://extensions` has no `chrome.action` and `developerPrivate`
/// reports no action state at all. The popup page the app mounts for a click is
/// one such page, so the shape this is meant for is: mount the popup view,
/// read the state before showing it, and open nothing when `popupUrl` is `""`.
///
/// The state is per tab, and the tab is Chromium's own active one rather than
/// anything this framework names: `cef_browser_t::get_identifier` claims in its
/// header to be the extension tab id and under Chrome style it is not, so a
/// `<webview>` has no tab id an app could pass. The answer carries the `tabId`
/// and `tabUrl` it was read for.
export function readExtensionAction(node: NdNodeRef<"webview">): Promise<ExtensionActionState> {
  return request(node, pendingActionState, "ext", "readExtensionAction", {});
}

/// Clicks an extension's action the way Chrome's toolbar button does, on the
/// tab `node` shows: Chromium opens the popup when the action has one for that
/// tab, and otherwise dispatches `action.onClicked` (`browserAction.onClicked`
/// for MV2) with the tab and grants `activeTab` on it. An app that mounts its
/// own popup view calls this only when `readExtensionAction` reports
/// `popupUrl: ""`, because a popup Chromium opens has no toolbar to hang from.
///
/// Chromium engine under Chrome style, on macOS and in the Views-hosted Linux
/// embedding (`ND_CEF_VIEWS_HOSTED=1`). Elsewhere it rejects.
export function triggerExtensionAction(node: NdNodeRef<"webview">, extensionId: string): Promise<void> {
  const id = nextId("ext");
  return new Promise<void>((resolve, reject) => {
    pendingTriggers.set(id, { resolve, reject });
    sendCommand(node, "triggerExtensionAction", { id, extensionId });
  });
}

/// The actions the installed extensions declare, for an app that draws its own
/// toolbar. Same view as `listExtensions`; requires the `onExtensionActions`
/// prop. A click is `triggerExtensionAction` when the live popup is `""`, and
/// otherwise a `<webview>` the app mounts at the popup's URL.
export function listExtensionActions(node: NdNodeRef<"webview">): Promise<ExtensionAction[]> {
  return request(node, pendingActions, "ext", "listExtensionActions", {});
}

/// Pass as a <webview>'s `onExtensionActions` prop.
export function onExtensionActions(e: { data: unknown }): void {
  const result = e.data as {
    id: string;
    ok: boolean;
    actions?: ExtensionAction[];
    action?: ExtensionActionState;
    triggered?: string;
    error?: string;
  };
  const trigger = pendingTriggers.get(result.id);
  if (trigger) {
    pendingTriggers.delete(result.id);
    if (result.ok) trigger.resolve();
    else trigger.reject(new Error(result.error ?? "triggerExtensionAction failed"));
    return;
  }
  const state = pendingActionState.get(result.id);
  if (state) {
    pendingActionState.delete(result.id);
    if (result.ok && result.action) state.resolve(result.action);
    else state.reject(new Error(result.error ?? "readExtensionAction failed"));
    return;
  }
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

// ============================================================================
// Downloads
// ============================================================================

/// Answers Chrome's own "Add <name>?" prompt for the next Web Store install,
/// unseen. For an app that confirms a store install in a dialog of its own and
/// then lets the store page go on: the first Chromium dialog that comes up in
/// the next 20 seconds is taken as the prompt and accepted. Chromium engine,
/// Chrome style only; elsewhere it does nothing.
export function acceptExtensionInstall(node: NdNodeRef<"webview">): void {
  sendCommand(node, "acceptExtensionInstall", {});
}

/// How a page asked for the window in `onNewWindow`. Chrome shows a
/// foreground tab next to the page that opened it and puts a background one
/// after that page's other new tabs; "window" and "popup" are what Chrome
/// would give a window of its own.
export type NewWindowDisposition = "foregroundTab" | "backgroundTab" | "window" | "popup";

export interface NewWindowRequest {
  url: string;
  /// Absent on the system engine, which reports the URL only.
  disposition?: NewWindowDisposition;
  userGesture?: boolean;
  /// A tab Chrome made on its own (an extension's `chrome.tabs.create`, the
  /// page an extension opens on install) rather than one the page asked for:
  /// Chrome adds it at the end of the strip.
  fromExtension?: boolean;
}

/// Reads an `onNewWindow` event. The Chromium engine adds the disposition
/// beside the URL in `data`.
export function newWindowRequest(e: { text: string }): NewWindowRequest {
  const data = (e as { data?: Omit<NewWindowRequest, "url"> }).data;
  return { url: e.text, disposition: data?.disposition, userGesture: data?.userGesture, fromExtension: data?.fromExtension };
}

/// `onDownloadRequested`. With an `id` (Chromium) the engine holds the
/// download until `respondDownload` names where it goes; without one (the
/// system engine) the engine has dropped it and the app fetches `url` itself.
export interface DownloadRequest {
  id?: string;
  url: string;
  suggestedFilename?: string;
}

export type DownloadState = "running" | "done" | "failed" | "cancelled";

/// `onDownloadUpdated`, for a download the app gave a path. `total` is -1
/// while the size is unknown and `speed` is in bytes per second. A paused
/// download is still `running`, with `paused` set.
export interface DownloadUpdate {
  id: string;
  state: DownloadState;
  received: number;
  total: number;
  path: string;
  speed?: number;
  paused?: boolean;
}

/// Answers a `downloadRequested` that carried an `id`: the engine runs the
/// transfer to `path`, a full file path. No `path` cancels it.
export function respondDownload(node: NdNodeRef<"webview">, id: string, path?: string): void {
  sendCommand(node, "respondDownload", path ? { id, path } : { id });
}

/// Downloads `url` with this view's profile and cookies. It comes back as
/// `downloadRequested` like a download the page started.
export function startDownload(node: NdNodeRef<"webview">, url: string): void {
  sendCommand(node, "startDownload", { url });
}

/// Pause, resume and cancel take a running download's id. Any live view of
/// the same engine can carry them: a download is the engine's, not the tab's.
/// Resume also picks up a `failed` download where it stopped, when the server
/// allows it.
export function pauseDownload(node: NdNodeRef<"webview">, id: string): void {
  sendCommand(node, "pauseDownload", { id });
}

export function resumeDownload(node: NdNodeRef<"webview">, id: string): void {
  sendCommand(node, "resumeDownload", { id });
}

export function cancelDownload(node: NdNodeRef<"webview">, id: string): void {
  sendCommand(node, "cancelDownload", { id });
}
