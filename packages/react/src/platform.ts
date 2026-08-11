// Platform identity for a running NativeDesktop app.
//
// Two independent axes, because they can disagree: `os` is where the process
// runs; `backend` is which native widget layer is actually drawing. The GTK
// backend also runs on macOS (via GTK's Quartz gdk), so `os === "macos"` does
// NOT imply AppKit — branch on `backend` for renderer-specific quirks, on `os`
// for OS conventions (paths, keybindings, menu placement).
//
// `backend` is authoritative from the host: it arrives in the NDP helloAck and
// the renderer installs it (setBackend) before your tree mounts, so reading it
// inside a component, effect, or handler is always safe. It is "unknown" only
// before render()'s handshake completes. `os` derives from the Bun child's own
// `process.platform`, which is co-located with the host.

import { intrinsicToName, widgetCommands } from "./generated/schema-meta.ts";

export type Backend = "gtk" | "appkit" | "unknown";
export type OS = "macos" | "linux" | "windows";

let backend: Backend = "unknown";

/** Renderer-internal: called once from `render()` after the handshake. */
export function setBackend(name: string): void {
  backend = name as Backend;
}

// The host build's capability manifest (helloAck hostWidgets/hostCommands).
// null = the host predates the fields; hasWidget/hasCommand then fall back
// to this runtime's own generated schema tables — exactly the pre-manifest
// behavior, where JS-schema knowledge was the only answer available.
let hostWidgets: Set<string> | null = null;
let hostCommands: Set<string> | null = null;

/** Renderer-internal: called once from `render()` beside `setBackend`. */
export function setHostManifest(widgets: Set<string> | null, commands: Set<string> | null): void {
  hostWidgets = widgets;
  hostCommands = commands;
}

/**
 * True when the connected host build knows the intrinsic (e.g.
 * `hasWidget("sourcetree")`). Answers from the host's handshake manifest;
 * against an older host that doesn't send one, falls back to this runtime's
 * own schema table (i.e. "the JS side knows it").
 */
export function hasWidget(type: string): boolean {
  if (hostWidgets) return hostWidgets.has(type);
  return type in intrinsicToName;
}

/**
 * True when the connected host build dispatches `command` on `type` (e.g.
 * `hasCommand("window", "present")`). Same manifest-with-fallback semantics
 * as `hasWidget`. Replaces try/catch around `sendCommand` for feature
 * detection.
 */
export function hasCommand(type: string, command: string): boolean {
  if (hostCommands) return hostCommands.has(`${type}.${command}`);
  return (widgetCommands[type] ?? []).includes(command);
}

function currentOS(): OS {
  switch (process.platform) {
    case "darwin":
      return "macos";
    case "win32":
      return "windows";
    default:
      return "linux";
  }
}

export const Platform = {
  /** The native widget backend drawing this app: "gtk" | "appkit". */
  get backend(): Backend {
    return backend;
  },
  /** The OS this app is running on. */
  get os(): OS {
    return currentOS();
  },
  /**
   * Pick a value by the active backend, like React Native's `Platform.select`.
   * Returns `default` when the active backend has no matching entry.
   */
  select<T>(spec: Partial<Record<Backend, T>> & { default?: T }): T | undefined {
    return backend in spec ? spec[backend] : spec.default;
  },
};
