import ReconcilerFactory from "react-reconciler";
import { ConcurrentRoot } from "react-reconciler/constants";
import type { ReactNode, ReactPortal } from "react";
import { connect, installErrorHandlers, isHot, reportRenderError } from "@nativedesktop/core";
import { hostConfig, bindCommitTargets, setPriorityFor, type Container } from "./host-config.ts";
import { getHmrState, setHmrState, setupRefresh, registerRoot, hotUpdateRoot } from "./hmr.ts";

type ReconcilerInstance = {
  createContainer: (...a: unknown[]) => unknown;
  updateContainer: (...a: unknown[]) => void;
  setRefreshHandler?: (h: unknown) => void;
  injectIntoDevTools?: () => unknown;
};

// `bun --hot` re-runs this module's top-level statements (render()'s caller,
// e.g. `await render(<App/>)`) on every edit in the module graph — connect +
// mount must therefore be idempotent (guarded by a globalThis singleton).
// First boot connects, handshakes, and creates the reconciler root; every
// subsequent call (a hot re-eval) reuses the surviving root instead.
export async function render(element: ReactNode): Promise<void> {
  installErrorHandlers();
  let state = getHmrState();
  if (!state) {
    const session = await connect({
      beforeEvent: (e) => setPriorityFor((e.priority as "discrete" | "continuous" | "default") ?? "discrete"),
    });
    bindCommitTargets(session.batch, session.registry);
    const configWithFlush = {
      ...hostConfig,
      resetAfterCommit: () => session.commit(),
    };

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
      // onUncaughtError: report, then rethrow. React re-raises the throw in
      // a setTimeout, so the process-level uncaughtException handler (which
      // dedupes via errors.ts's renderFatal mark) performs the actual exit.
      (e: unknown, info?: { componentStack?: string }) => {
        reportRenderError(e, "renderUncaught", info?.componentStack);
        throw e;
      },
      (e: unknown, info?: { componentStack?: string }) => {
        reportRenderError(e, "renderCaught", info?.componentStack);
      },
      (e: unknown, info?: { componentStack?: string }) => {
        reportRenderError(e, "renderRecoverable", info?.componentStack);
      },
      null,
    );
    state = { root, reconciler: Reconciler, bootCount: 0 };
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
    // remount (all hook state reset) instead of updating.
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
