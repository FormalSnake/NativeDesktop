import ReconcilerFactory from "react-reconciler";
import { ConcurrentRoot } from "react-reconciler/constants";
import type { ReactNode } from "react";
import { Ndp, type EventMsg } from "../../../runtime/ndp.ts";
import { hostConfig, bindCommitTargets, setPriorityFor, type Container } from "./host-config.ts";
import { Batch, NodeRegistry } from "./ops.ts";
import { currentGeneration } from "./ids.ts";
import { getHmrState, setHmrState, isHot, installErrorReporting, setupRefresh } from "./hmr.ts";

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
  state.reconciler.updateContainer(element, state.root, null, () => {});

  // Keep the process alive so the reconciler's scheduler + event stream run.
  // Only the first boot awaits this — a hot re-eval must return so the
  // re-run entry doesn't pile up a second forever-pending promise.
  if (state.bootCount === 1) await new Promise<void>(() => {});
}
