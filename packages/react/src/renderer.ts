import ReconcilerFactory from "react-reconciler";
import { ConcurrentRoot } from "react-reconciler/constants";
import type { ReactNode } from "react";
import { Ndp, type EventMsg } from "../../../runtime/ndp.ts";
import { hostConfig, bindCommitTargets, setPriorityFor, type Container } from "./host-config.ts";
import { Batch, NodeRegistry } from "./ops.ts";
import { currentGeneration } from "./ids.ts";

export async function render(element: ReactNode): Promise<void> {
  const ndp = await Ndp.connect();
  await ndp.handshake({ name: "bun", version: Bun.version });

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
    const rec = registry.get(e.nodeId);
    if (e.name === "clicked") rec?.onClick?.();
  });

  const Reconciler = (ReconcilerFactory as unknown as (c: typeof configWithFlush) => {
    createContainer: (...a: unknown[]) => unknown;
    updateContainer: (...a: unknown[]) => void;
  })(configWithFlush);
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
  Reconciler.updateContainer(element, root, null, () => {});

  // Keep the process alive so the reconciler's scheduler + event stream run.
  await new Promise<void>(() => {});
}
