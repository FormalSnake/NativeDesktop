// `bun --hot`-safe HMR state (M8). Verified this session: `bun --hot` keeps
// the same OS pid + unix socket across edits, but re-runs every top-level
// statement in the module graph on any edit (see docs/superpowers/plans —
// M8 empirically-verified findings). Module-local `let`/`const` bindings are
// re-initialized on each re-eval; only `globalThis`-keyed state survives.
// `render()` must therefore stash its connect+mount state here so a hot
// re-eval reuses the live Ndp/root instead of double-connecting.

import type { Ndp } from "../../../runtime/ndp.ts";
import * as RefreshRuntimeFirstEval from "react-refresh/runtime";
import { newGeneration } from "./ids.ts";

// react-refresh/runtime keeps its family registry (allFamiliesByID,
// mountedRoots, helpersByRoot, ...) in module-local closures, not on any
// object it hands out — verified by reading its bundled source this
// session. A fresh `import * as RefreshRuntime from "react-refresh/runtime"`
// therefore starts every one of those maps EMPTY, same as any other module
// under `--hot`'s full-graph re-eval (ids.ts's generation counter is the
// same shape of reset, just intentional there). Unlike `react`/
// `react-reconciler`, this package has NO internal `require()`s of its own
// (verified — grepped its bundled cjs), so it can be pinned via a plain
// globalThis stash entirely within this module, no Bun.plugin/preload
// aliasing needed (that mechanism was tried and abandoned for react/
// react-reconciler — see dev-react.ts's header — because it collides with
// their internal `require()`s; this package has none, so the collision
// doesn't apply). Pinning it is required for register()/performReactRefresh
// to ever match an old family against a new one: the "old" family must have
// been registered by the SAME runtime instance that "new" registration and
// performReactRefresh() run against.
declare global {
  // eslint-disable-next-line no-var
  var __nd_refresh_runtime: typeof RefreshRuntimeFirstEval | undefined;
}
function pinnedRefreshRuntime(): typeof RefreshRuntimeFirstEval {
  if (!globalThis.__nd_refresh_runtime) globalThis.__nd_refresh_runtime = RefreshRuntimeFirstEval;
  return globalThis.__nd_refresh_runtime;
}

export interface HmrState {
  ndp: Ndp;
  root: unknown;
  reconciler: { updateContainer: (...a: unknown[]) => void };
  bootCount: number;
}

declare global {
  // eslint-disable-next-line no-var
  var __nd_hmr: HmrState | undefined;
}

export function getHmrState(): HmrState | undefined {
  return globalThis.__nd_hmr;
}

export function setHmrState(s: HmrState): void {
  globalThis.__nd_hmr = s;
}

export function isHot(): boolean {
  return process.env.ND_DEV === "1";
}

/** Reports an uncaught exception / unhandled rejection to the host before
 *  the process exits, so the crash overlay shows the real error. */
export function installErrorReporting(ndp: Ndp): void {
  const report = (err: unknown): void => {
    const e = err instanceof Error ? err : new Error(String(err));
    ndp.sendRuntimeError(e.message, e.stack ?? "");
  };
  process.on("uncaughtException", (err) => {
    report(err);
    process.exit(1);
  });
  process.on("unhandledRejection", (reason) => {
    report(reason);
    process.exit(1);
  });
}

interface RefreshableReconciler {
  setRefreshHandler?: (h: unknown) => void;
  // Registers the reconciler's commit/schedule hooks with the object at
  // globalThis.__REACT_DEVTOOLS_GLOBAL_HOOK__ (installed by
  // injectIntoGlobalHook below). Without this call, react-refresh's
  // `mountedRoots` stays empty and performReactRefresh() is a silent no-op —
  // verified empirically this session (a probe with everything else wired
  // produced updatedFamilies.size === 1 but no re-render without this call).
  injectIntoDevTools?: () => unknown;
}

let refreshReady = false;

/** Wires react-refresh's runtime into the reconciler (M8-D2 primary path).
 *  `injectIntoGlobalHook`/`injectIntoDevTools`/`setRefreshHandler`/manual
 *  `$RefreshReg$`/`$RefreshSig$` must all be installed before any app React
 *  code runs — render() calls this before createContainer, and the app
 *  module's import of @nativedesktop/react runs this module first, so
 *  ordering holds. Bun's `--hot` does NOT inject `$RefreshReg$`/`$RefreshSig$`
 *  (verified this session), so they are installed manually here. */
export function setupRefresh(reconciler: RefreshableReconciler): void {
  if (refreshReady) return;
  // The upstream type targets a browser `Window`; react-refresh's runtime
  // only reads/writes a hook property on whatever object it's given, so a
  // Bun `globalThis` works identically at runtime.
  pinnedRefreshRuntime().injectIntoGlobalHook(globalThis as unknown as Window);
  (globalThis as Record<string, unknown>).$RefreshReg$ = (type: unknown, id: string): void => {
    pinnedRefreshRuntime().register(type, id);
  };
  (globalThis as Record<string, unknown>).$RefreshSig$ = (): unknown =>
    pinnedRefreshRuntime().createSignatureFunctionForTransform();
  reconciler.setRefreshHandler?.((type: unknown) => pinnedRefreshRuntime().getFamilyByType(type));
  // Must run AFTER injectIntoGlobalHook (the devtools hook must exist first)
  // and AFTER setRefreshHandler (injectIntoDevTools snapshots scheduleRefresh
  // etc. into the internals object it hands to the hook's inject()).
  reconciler.injectIntoDevTools?.();
  refreshReady = true;
}

/** Call from the app entry after a hot re-eval re-runs its modules. */
export function performRefresh(): void {
  pinnedRefreshRuntime().performReactRefresh();
}

/** Registers a module's exported components by name — the babel Fast Refresh
 *  transform's job, done manually because Bun's `--hot` does not run it
 *  (M8-D2 / verified finding). Call once per hot re-eval, before
 *  performRefresh(). */
export function registerExports(mod: Record<string, unknown>, moduleId: string): void {
  for (const [name, val] of Object.entries(mod)) {
    if (typeof val === "function" && /^[A-Z]/.test(name)) {
      pinnedRefreshRuntime().register(val, `${moduleId} ${name}`);
    }
  }
}

// Fixed family key for the app's root element's type. There is exactly one
// render() root per process, so a single stable key (rather than a
// module-id-derived one) is enough to let react-refresh match the OLD root
// component type (registered on the previous eval) against the NEW one
// (registered on this eval) and patch the live fiber in place instead of
// remounting it — verified empirically this session: calling
// `reconciler.updateContainer(newElement, ...)` again on a hot re-eval
// resets all hook state (the new element's `.type` is a different function
// reference than the old one, so the reconciler treats it as a type change,
// not an update) even though `react`/`react-reconciler` module identity is
// correctly pinned (dev-react.ts) — pinning identity alone does not give
// state-preserving re-renders; react-refresh's family-based fiber patch is
// what actually does that.
const ROOT_FAMILY_KEY = "nd-root";

/** Called by render() on first boot, before the first updateContainer, so a
 *  family exists for the NEXT eval's hotUpdateRoot() call to match against. */
export function registerRoot(rootComponentType: unknown): void {
  pinnedRefreshRuntime().register(rootComponentType, ROOT_FAMILY_KEY);
}

/** Called by render() on every hot re-eval (never on first boot). Registers
 *  the new root element's type under the SAME family key used on the
 *  previous eval, then asks react-refresh to patch the live fiber tree in
 *  place — this is what makes a hot edit state-preserving instead of a full
 *  remount. Must NOT be combined with a normal updateContainer call on the
 *  same eval (see above). */
export function hotUpdateRoot(rootComponentType: unknown): void {
  pinnedRefreshRuntime().register(rootComponentType, ROOT_FAMILY_KEY);
  pinnedRefreshRuntime().performReactRefresh();
}

/** Bumps the generation counter (ids.ts) so the next commit's CommitBatch
 *  carries a higher generation, triggering the host's generation GC (Task 3)
 *  of orphaned widgets. Used for structural / non-refreshable edits that
 *  react-refresh cannot preserve state across. */
export function fullReload(): void {
  newGeneration();
}
