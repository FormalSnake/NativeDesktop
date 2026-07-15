// Promise-based bindings for the "system capabilities" API (M15): native
// dialogs, clipboard, notifications, recent documents, credentials, and
// app-level lifecycle/OS events (activate/deactivate, open-url, open-file,
// file drop). Every method below rides a systemRequest/systemResponse round
// trip over NDP (runtime/ndp.ts's `request()`); every `on*` subscribes to a
// systemEvent channel. Methods are gated host-side by ACL — dialog/
// notification/recent/clipboard.writeText/audio.* are default-granted;
// readText and credentials.* are default-denied and reject with "capability
// denied" unless the app's ND_ACL_GRANTS manifest lists them.

import { getHmrState } from "./hmr.ts";

function requireHost(name: string): void {
  if (!getHmrState()) throw new Error(`${name}() used before render(): no host connection yet`);
}

function call(method: string, params: unknown = {}): Promise<unknown> {
  requireHost(method);
  return getHmrState()!.ndp.request(method, params);
}

// --- systemEvent registry ---------------------------------------------------
// A channel -> handler-set map, stashed on globalThis (same pattern as
// hmr.ts's __nd_hmr) so subscriptions survive a `bun --hot` re-eval of this
// module instead of resetting to an empty registry mid-session.
type SystemEventHandler = (data: unknown) => void;

declare global {
  // eslint-disable-next-line no-var
  var __nd_system_events: Map<string, Set<SystemEventHandler>> | undefined;
}

function registry(): Map<string, Set<SystemEventHandler>> {
  if (!globalThis.__nd_system_events) globalThis.__nd_system_events = new Map();
  return globalThis.__nd_system_events;
}

function subscribe(channel: string, name: string, handler: SystemEventHandler): () => void {
  requireHost(name);
  let handlers = registry().get(channel);
  if (!handlers) registry().set(channel, (handlers = new Set()));
  handlers.add(handler);
  return () => handlers!.delete(handler);
}

/** Wired up by renderer.ts from `ndp.onSystemEvent()` — not part of the public API. */
export function dispatchSystemEvent(channel: string, data: unknown): void {
  for (const handler of registry().get(channel) ?? []) handler(data);
}

// --- option / filter types ---------------------------------------------------

/** A native file-picker filter, e.g. `{ name: "Images", extensions: ["png", "jpg"] }`. */
export interface FileFilter {
  name: string;
  extensions: string[];
}

export interface OpenFileOptions {
  title?: string;
  defaultPath?: string;
  filters?: FileFilter[];
  multiple?: boolean;
  directories?: boolean;
}

export interface SaveFileOptions {
  title?: string;
  defaultPath?: string;
  defaultName?: string;
  filters?: FileFilter[];
}

export type MessageLevel = "info" | "warning" | "error";

export interface MessageOptions {
  message: string;
  detail?: string;
  level?: MessageLevel;
  buttons?: string[];
  defaultButton?: number;
}

export interface NotificationOptions {
  title: string;
  body?: string;
}

// --- dialogs -----------------------------------------------------------------

/** Native file/message dialogs. Default-granted (no ACL manifest entry needed). */
export const dialog = {
  /** Opens a native "choose file(s)" dialog. Resolves to the chosen paths, or `[]` if the user cancels. */
  openFile(options: OpenFileOptions = {}): Promise<string[]> {
    return call("dialog.openFile", options) as Promise<string[]>;
  },
  /** Opens a native "save file" dialog. Resolves to the chosen path, or `null` if the user cancels. */
  saveFile(options: SaveFileOptions = {}): Promise<string | null> {
    return call("dialog.saveFile", options) as Promise<string | null>;
  },
  /** Shows a native modal alert. Resolves to the 0-based index of the button the user clicked. */
  showMessage(options: MessageOptions): Promise<number> {
    return call("dialog.showMessage", options) as Promise<number>;
  },
};

// --- clipboard -----------------------------------------------------------------

export const clipboard = {
  /** Reads text from the system clipboard. Default-denied — needs `core:clipboard.readText` in ND_ACL_GRANTS, or rejects with "capability denied". */
  readText(): Promise<string> {
    return call("clipboard.readText") as Promise<string>;
  },
  /** Writes text to the system clipboard. Default-granted. */
  async writeText(text: string): Promise<void> {
    await call("clipboard.writeText", { text });
  },
};

// --- notifications -----------------------------------------------------------------

export const notifications = {
  /** Shows a native OS notification. Resolves to the notification's id (correlates a later `notification.click` event). Default-granted. */
  show(options: NotificationOptions): Promise<string> {
    return call("notification.show", options) as Promise<string>;
  },
  /** Subscribes to notification-click events. Returns an unsubscribe function, usable directly as a `useEffect` cleanup. */
  onClick(handler: (id: string) => void): () => void {
    return subscribe("notification.click", "notifications.onClick", (data) => handler((data as { id: string }).id));
  },
};

// --- recent documents -----------------------------------------------------------------

export const recentDocuments = {
  /** Adds `path` to the OS's recent-documents list. Default-granted. */
  async add(path: string): Promise<void> {
    await call("recent.add", { path });
  },
  /** Clears the OS's recent-documents list. Default-granted. */
  async clear(): Promise<void> {
    await call("recent.clear");
  },
};

// --- credentials -----------------------------------------------------------------

/** OS keychain / credential storage. Default-denied — needs `core:credentials.*` in ND_ACL_GRANTS, or these reject with "capability denied". */
export const credentials = {
  /** Stores `secret` under (`service`, `account`) in the OS credential store. */
  async set(service: string, account: string, secret: string): Promise<void> {
    await call("credentials.set", { service, account, secret });
  },
  /** Reads the secret stored under (`service`, `account`), or `null` if none exists. */
  get(service: string, account: string): Promise<string | null> {
    return call("credentials.get", { service, account }) as Promise<string | null>;
  },
  /** Removes the secret stored under (`service`, `account`). */
  async delete(service: string, account: string): Promise<void> {
    await call("credentials.delete", { service, account });
  },
};

// --- app lifecycle / OS events -----------------------------------------------------------------

export const app = {
  /** Subscribes to app-activation (e.g. Dock/taskbar re-activation). Returns an unsubscribe function. */
  onActivate(handler: () => void): () => void {
    return subscribe("app.activate", "app.onActivate", () => handler());
  },
  /** Subscribes to app-deactivation. Returns an unsubscribe function. */
  onDeactivate(handler: () => void): () => void {
    return subscribe("app.deactivate", "app.onDeactivate", () => handler());
  },
  /** Subscribes to OS "open URL" launch requests (e.g. a registered custom URL scheme). Returns an unsubscribe function. */
  onOpenUrl(handler: (url: string) => void): () => void {
    return subscribe("app.openUrl", "app.onOpenUrl", (data) => handler((data as { url: string }).url));
  },
  /** Subscribes to OS "open file" launch requests (e.g. double-clicking a registered document type). Returns an unsubscribe function. */
  onOpenFile(handler: (paths: string[]) => void): () => void {
    return subscribe("app.openFile", "app.onOpenFile", (data) => handler((data as { paths: string[] }).paths));
  },
  /** Subscribes to files dropped onto a window. Returns an unsubscribe function. */
  onFileDrop(handler: (e: { paths: string[]; windowId: number }) => void): () => void {
    return subscribe("fileDrop", "app.onFileDrop", (data) => handler(data as { paths: string[]; windowId: number }));
  },
};

// --- audio -----------------------------------------------------------------

export interface AudioPlayOptions {
  path?: string;
  url?: string;
  volume?: number;
  spectrum?: boolean;
}

export type AudioState = "playing" | "paused" | "ended" | "stopped" | "error";

export interface AudioStateEvent {
  handle: string;
  state: AudioState;
  position: number;
  duration: number | null;
  error?: string;
}

export interface AudioSpectrumEvent {
  handle: string;
  bins: number[];
}

/** Audio playback. `core:audio` is default-granted (ND_ACL_GRANTS manifests extend the default set — they cannot revoke it). */
export const audio = {
  /** Starts playback from exactly one of `path` (local file) or `url` (remote). Resolves to a handle used by the other methods below. */
  play(options: AudioPlayOptions): Promise<string> {
    return call("audio.play", options) as Promise<string>;
  },
  /** Pauses playback for `handle`. */
  async pause(handle: string): Promise<void> {
    await call("audio.pause", { handle });
  },
  /** Resumes playback for `handle`. */
  async resume(handle: string): Promise<void> {
    await call("audio.resume", { handle });
  },
  /** Stops playback and releases `handle` — it is no longer valid afterward. */
  async stop(handle: string): Promise<void> {
    await call("audio.stop", { handle });
  },
  /** Seeks `handle` to `positionMs` milliseconds. */
  async seek(handle: string, positionMs: number): Promise<void> {
    await call("audio.seek", { handle, position: positionMs });
  },
  /** Sets playback volume (0..1) for `handle`. */
  async setVolume(handle: string, volume: number): Promise<void> {
    await call("audio.setVolume", { handle, volume });
  },
  /** Subscribes to playback state transitions (not position ticks). Returns an unsubscribe function. */
  onState(handler: (e: AudioStateEvent) => void): () => void {
    return subscribe("audio.state", "audio.onState", (data) => handler(data as AudioStateEvent));
  },
  /** Subscribes to spectrum frames (32 bins, 0..1, ~15 Hz) — only emitted for handles played with `spectrum: true`. Returns an unsubscribe function. */
  onSpectrum(handler: (e: AudioSpectrumEvent) => void): () => void {
    return subscribe("audio.spectrum", "audio.onSpectrum", (data) => handler(data as AudioSpectrumEvent));
  },
};
