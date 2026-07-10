// `bun --hot`-safe HMR state (M8). Verified this session: `bun --hot` keeps
// the same OS pid + unix socket across edits, but re-runs every top-level
// statement in the module graph on any edit (see docs/superpowers/plans —
// M8 empirically-verified findings). Module-local `let`/`const` bindings are
// re-initialized on each re-eval; only `globalThis`-keyed state survives.
// `render()` must therefore stash its connect+mount state here so a hot
// re-eval reuses the live Ndp/root instead of double-connecting.

import type { Ndp } from "../../../runtime/ndp.ts";

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

/** No-op placeholder; Task 2 replaces this with the react-refresh wiring. */
export function setupRefresh(_reconciler: unknown): void {}
