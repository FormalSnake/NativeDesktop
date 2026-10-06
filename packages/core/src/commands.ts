import type { NdNodeRef, WidgetType } from "./generated/widgets.ts";
import { widgetCommands, type WidgetCommandNames } from "./generated/schema-meta.ts";
import { hasCommand } from "./platform.ts";
import { getSession, isHot } from "./session.ts";

function dispatchWidgetCommand(caller: string, node: NdNodeRef, command: string, arg: unknown): void {
  const session = getSession();
  if (!session) throw new Error(`${caller}() before render(): no NDP connection yet`);
  session.ndp.sendWidgetCommand(node.id, command, arg ?? null);
}

const warnedHostUnknownCommand = new Set<string>();

/// Sends an imperative command to a mounted widget (widgetCommand NDP frame).
/// `node` is what a host-element `ref` resolves to. Command names are
/// schema-typed per widget (WidgetCommandNames) and validated again at runtime
/// so a stale string fails loudly here, not silently host-side.
export function sendCommand<T extends keyof WidgetCommandNames & WidgetType>(
  node: NdNodeRef<T>,
  command: WidgetCommandNames[T],
  arg?: unknown,
): void {
  const allowed = widgetCommands[node.type] ?? [];
  if (!allowed.includes(command)) {
    throw new Error(`<${node.type}> does not accept command "${command}" (valid: ${allowed.join(", ") || "none"})`);
  }
  // JS-known but host-unknown (an older host build): still sent (the host
  // logs and drops it), but warn once in dev.
  if (isHot() && !hasCommand(node.type, command)) {
    const key = `${node.type}.${command}`;
    if (!warnedHostUnknownCommand.has(key)) {
      warnedHostUnknownCommand.add(key);
      console.warn(`sendCommand: the connected host does not dispatch "${key}"; gate it with hasCommand("${node.type}", "${command}").`);
    }
  }
  dispatchWidgetCommand("sendCommand", node, command, arg);
}

/// Sends an imperative command to an app-owned <nativeview>. Command names
/// are plugin-defined (native-module ABI), not schema-validated; only a
/// non-empty string is required, and the host resolves it.
export function sendNativeCommand(node: NdNodeRef<"nativeview">, command: string, arg?: unknown): void {
  if (!command) throw new Error("sendNativeCommand() requires a non-empty command");
  dispatchWidgetCommand("sendNativeCommand", node, command, arg);
}

/// Moves a live node's native widget under `toParent` (optionally before
/// `before`) WITHOUT destroying it: the widget-preserving cross-window move.
/// The node MUST stay mounted at a stable position in the renderer's tree
/// (typically inside a portal into a pool) so the renderer never unmounts it;
/// this relocates only the native widget, preserving a <webview>'s loaded
/// page, scroll and JS state that unmount+remount would lose. Rides the
/// widgetCommand frame with a reserved command, so no protocol change.
export function moveNode(node: NdNodeRef, toParent: NdNodeRef, before?: NdNodeRef | null): void {
  dispatchWidgetCommand("moveNode", node, "__ndReparent", { parent: toParent.id, before: before?.id ?? null });
}
