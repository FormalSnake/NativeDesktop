import ReconcilerFactory from "react-reconciler";
import { ConcurrentRoot } from "react-reconciler/constants";
import type { ReactNode } from "react";
import { Ndp, type EventMsg } from "../../../runtime/ndp.ts";
import type { NdNodeRef, WidgetType } from "./generated/intrinsics.ts";
import { widgetCommands, type WidgetCommandNames } from "./generated/schema-meta.ts";
import { hostConfig, bindCommitTargets, setPriorityFor, type Container } from "./host-config.ts";
import { Batch, NodeRegistry } from "./ops.ts";
import { currentGeneration } from "./ids.ts";
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

/// Sends an imperative command to a mounted widget (widgetCommand NDP frame,
/// M14). `node` is what a host-element `ref` resolves to — e.g.
/// `const wv = useRef<NdNodeRef<"webview">>(null)` then
/// `sendCommand(wv.current!, "goBack")`. Command names are schema-typed per
/// widget (WidgetCommandNames) and validated again at runtime so a stale
/// string fails loudly here, not silently host-side.
export function sendNativeCommand(node: NdNodeRef<"nativeview">, command: string, arg?: unknown): void {
  const state = getHmrState();
  if (!state) throw new Error("sendNativeCommand() before render(): no NDP connection yet");
  if (!command) throw new Error("sendNativeCommand() requires a non-empty command");
  state.ndp.sendWidgetCommand(node.id, command, arg ?? null);
}

export function sendCommand<T extends keyof WidgetCommandNames & WidgetType>(
  node: NdNodeRef<T>,
  command: WidgetCommandNames[T],
  arg?: unknown,
): void {
  const state = getHmrState();
  if (!state) throw new Error("sendCommand() before render(): no NDP connection yet");
  const allowed = widgetCommands[node.type] ?? [];
  if (!allowed.includes(command)) {
    throw new Error(`<${node.type}> does not accept command "${command}" (valid: ${allowed.join(", ") || "none"})`);
  }
  state.ndp.sendWidgetCommand(node.id, command, arg ?? null);
}
