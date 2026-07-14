import ReconcilerFactory from "react-reconciler";
import { ConcurrentRoot } from "react-reconciler/constants";
import type { ReactNode, ReactPortal } from "react";
import { Ndp, type EventMsg } from "../../../runtime/ndp.ts";
import type { NdNodeRef, WidgetType } from "./generated/intrinsics.ts";
import { widgetCommands, type WidgetCommandNames } from "./generated/schema-meta.ts";
import { hostConfig, bindCommitTargets, setPriorityFor, type Container } from "./host-config.ts";
import { Batch, NodeRegistry } from "./ops.ts";
import { currentGeneration } from "./ids.ts";
import { setBackend } from "./platform.ts";
import { dispatchSystemEvent } from "./system.ts";
import {
  getHmrState,
  setHmrState,
  isHot,
  installErrorReporting,
  setupRefresh,
  registerRoot,
  hotUpdateRoot,
} from "./hmr.ts";

type ReconcilerInstance = {
  createContainer: (...a: unknown[]) => unknown;
  updateContainer: (...a: unknown[]) => void;
  setRefreshHandler?: (h: unknown) => void;
  injectIntoDevTools?: () => unknown;
};

// `bun --hot` re-runs this module's top-level statements (render()'s caller,
// e.g. `await render(<App/>)`) on every edit in the module graph — connect +
// mount must therefore be idempotent (M8, globalThis singleton guard). First
// boot connects, handshakes, and creates the reconciler root; every
// subsequent call (a hot re-eval) reuses the surviving root instead.
export async function render(element: ReactNode): Promise<void> {
  let state = getHmrState();
  if (!state) {
    const ndp = await Ndp.connect();
    await ndp.handshake({ name: "bun", version: Bun.version });
    setBackend(ndp.backend);
    installErrorReporting(ndp);

    const batch = new Batch();
    const registry = new NodeRegistry();
    bindCommitTargets(batch, registry);

    let commitId = 0;
    const configWithFlush = {
      ...hostConfig,
      resetAfterCommit() {
        const ops = batch.drain();
        if (ops.length) ndp.sendCommit({ commitId: commitId++, generation: currentGeneration(), ops });
      },
    };

    ndp.onEvent((e: EventMsg) => {
      setPriorityFor((e.priority as "discrete" | "continuous" | "default") ?? "discrete");
      registry.get(e.nodeId)?.handlers[e.name]?.(e.payload);
    });
    ndp.onSystemEvent((channel, data) => dispatchSystemEvent(channel, data));

    const Reconciler = (ReconcilerFactory as unknown as (c: typeof configWithFlush) => ReconcilerInstance)(
      configWithFlush,
    );
    if (isHot()) setupRefresh(Reconciler);
    const container: Container = { rootId: null };
    const root = Reconciler.createContainer(
      container,
      ConcurrentRoot,
      null,
      false,
      null,
      "nd",
      (e: unknown) => { throw e; },
      () => {},
      () => {},
      null,
    );
    state = { ndp, root, reconciler: Reconciler, bootCount: 0 };
    setHmrState(state);
  }

  state.bootCount += 1;
  if (state.bootCount === 1) {
    // registerRoot before the first commit so hotUpdateRoot's re-registration
    // on the NEXT eval has an existing family to match against (react-refresh
    // treats a register() with no prior entry for that id as a fresh
    // mount, not an update — see hmr.ts).
    if (isHot()) registerRoot((element as { type: unknown }).type);
    state.reconciler.updateContainer(element, state.root, null, () => {});
  } else if (isHot()) {
    // A hot re-eval must NOT call updateContainer with the new element
    // directly: `element.type` is a fresh function reference every re-eval,
    // so the reconciler would see a type change at the root and fully
    // remount (all hook state reset) instead of updating — verified
    // empirically this session against the real examples/counter app.
    // hotUpdateRoot() registers the new type under the SAME react-refresh
    // family as the previous eval's root and asks react-refresh to patch
    // the live fiber in place, which is what actually preserves state.
    hotUpdateRoot((element as { type: unknown }).type);
  } else {
    // Non-hot re-render call (not a --hot re-eval): a normal update.
    state.reconciler.updateContainer(element, state.root, null, () => {});
  }

  // Keep the process alive so the reconciler's scheduler + event stream run.
  // Only the first boot awaits this — a hot re-eval must return so the
  // re-run entry doesn't pile up a second forever-pending promise.
  if (state.bootCount === 1) await new Promise<void>(() => {});
}

function dispatchWidgetCommand(caller: string, node: NdNodeRef, command: string, arg: unknown): void {
  const state = getHmrState();
  if (!state) throw new Error(`${caller}() before render(): no NDP connection yet`);
  state.ndp.sendWidgetCommand(node.id, command, arg ?? null);
}

/// Sends an imperative command to a mounted widget (widgetCommand NDP frame,
/// M14). `node` is what a host-element `ref` resolves to — e.g.
/// `const wv = useRef<NdNodeRef<"webview">>(null)` then
/// `sendCommand(wv.current!, "goBack")`. Command names are schema-typed per
/// widget (WidgetCommandNames) and validated again at runtime so a stale
/// string fails loudly here, not silently host-side.
export function sendCommand<T extends keyof WidgetCommandNames & WidgetType>(
  node: NdNodeRef<T>,
  command: WidgetCommandNames[T],
  arg?: unknown,
): void {
  const allowed = widgetCommands[node.type] ?? [];
  if (!allowed.includes(command)) {
    throw new Error(`<${node.type}> does not accept command "${command}" (valid: ${allowed.join(", ") || "none"})`);
  }
  dispatchWidgetCommand("sendCommand", node, command, arg);
}

/// Sends an imperative command to an app-owned <nativeview>. Command names
/// are plugin-defined (native-module ABI), not schema-validated — only a
/// non-empty string is required; the host resolves it.
export function sendNativeCommand(node: NdNodeRef<"nativeview">, command: string, arg?: unknown): void {
  if (!command) throw new Error("sendNativeCommand() requires a non-empty command");
  dispatchWidgetCommand("sendNativeCommand", node, command, arg);
}

/// A stable, off-window host container that holds nodes which must survive being
/// moved between windows. Nodes rendered into a pool via `createPortal` become
/// DETACHED native widgets — created and kept alive, but shown in no window
/// until `moveNode` attaches them to a window's content. Its object identity is
/// what keeps React from ever tearing the subtree down (see `createPortal`).
export interface Pool {
  readonly rootId: null;
}

// One process-lifetime pool shared by `createPortal` when no explicit pool is
// given. A module constant so its identity is stable across every render — a
// fresh object each render would change the portal's container and force React
// to remount the subtree (react-reconciler's updatePortal keys on containerInfo
// identity), which is exactly what this whole mechanism exists to avoid.
const defaultPool: Pool = { rootId: null };

/// Creates an independent pool (see `Pool`) for apps that want more than one.
/// Call ONCE (module scope or a ref), never inside render — a new pool each
/// render changes the portal container and remounts the subtree, defeating the
/// point.
export function createPool(): Pool {
  return { rootId: null };
}

/// Renders `children` into `pool` (default: a shared process-lifetime pool)
/// instead of the enclosing window, while keeping their React fibers at THIS
/// position in the tree. React unmounts+remounts a subtree that moves to a
/// different parent — which the host turns into remove+create, so a <webview>
/// reloads and loses its page/scroll/JS state. A portal keeps the fiber's parent
/// fixed, so the subtree is NEVER torn down when the app "moves a tab"; pair it
/// with `moveNode` to relocate only the live NATIVE widget between windows.
/// Render the portal at a STABLE position (e.g. one per tab, keyed by tab id, at
/// the app root) so it outlives any single window.
export function createPortal(children: ReactNode, pool: Pool = defaultPool): ReactPortal {
  const state = getHmrState();
  if (!state) throw new Error("createPortal() before render(): no reconciler yet");
  const reconciler = state.reconciler as unknown as {
    createPortal: (children: ReactNode, containerInfo: unknown, implementation: unknown, key?: string | null) => ReactPortal;
  };
  return reconciler.createPortal(children, pool, null);
}

/// Moves a live node's native widget under `toParent` (optionally before
/// `before`) WITHOUT destroying it — the widget-preserving cross-window move.
/// The node MUST stay mounted at a stable React position (typically inside a
/// `createPortal(..., pool)`) so React never unmounts it; this relocates only
/// the native widget, preserving a <webview>'s loaded page / scroll / JS state
/// that re-parenting in the React tree (unmount+remount) would lose. `node` and
/// `toParent` are what a host-element `ref` resolves to. Rides the widgetCommand
/// frame with a reserved command, so no protocol/schema change is needed.
export function moveNode(node: NdNodeRef, toParent: NdNodeRef, before?: NdNodeRef | null): void {
  dispatchWidgetCommand("moveNode", node, "__ndReparent", { parent: toParent.id, before: before?.id ?? null });
}
