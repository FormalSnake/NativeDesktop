// Dev-mode hook-preserving re-export of `react`. Read this file's header
// before touching it — the design here is the outcome of two failed
// mechanisms, recorded so nobody re-attempts them:
//
// (a) `@nativedesktop/react` re-exporting react's hooks through a
//     `globalThis` stash covers OUR OWN JSX/APIs, but not the app's own
//     `import { useState } from "react"` — insufficient alone (that's
//     exactly what this file adds on top of).
// (b) Bun runtime aliasing of the bare `"react"`/`"react-reconciler"`
//     specifiers via `Bun.plugin`'s `builder.module()`, loaded through
//     `bun --hot --preload <script>` so the alias survives re-evals. This
//     DOES intercept real package names (`onResolve`/`onLoad` do NOT — they
//     never fire for specifiers Bun's own resolver can already resolve,
//     even for real installed packages) and the preload script genuinely
//     runs once per process, not per hot re-eval. It still fails: Bun
//     1.3.13 throws "Requested module is already fetched" (a native
//     `provideFetch` conflict, invisible to `require.cache` tampering) the
//     moment a `builder.module()`-aliased specifier is touched by BOTH an
//     ESM `import` and a CJS `require()` anywhere in the process.
//     `react-reconciler`'s bundled cjs does `require("react")`
//     (and jsx-runtime/jsx-dev-runtime do too), while this codebase's own
//     source and every app entry use genuine ESM `import`. That combination
//     is unavoidable here, so (b) cannot work without rewriting every
//     consumer to `require()` — which is a bigger, uglier convention change
//     than (c) below.
//
// (c) — what actually ships: `@nativedesktop/react` stashes the FIRST-eval
// `react` module instance in `globalThis` (same pattern hmr.ts already uses
// for the reconciler root) and re-exports ITS hooks. The reconciler created
// on first eval (renderer.ts, guarded by getHmrState()) already closes over
// this same first-eval `react` instance internally (react-reconciler's
// bundled cjs does `require("react")` once, when the factory is first
// invoked) — so a hook call resolved against the stashed instance resolves
// against the SAME dispatcher the live reconciler drives. A hook imported
// from the fresh, re-evaluated `react` module instead would resolve against
// ITS dispatcher, which is never set (no reconciler ever attaches to it) —
// exactly the crash this file exists to avoid.
//
// Convention this establishes: app code must import hooks from
// `@nativedesktop/react`, not `react` directly, to get HMR state
// preservation. Documented in docs/agents/README.md.
import * as ReactFirstEval from "react";

declare global {
  // eslint-disable-next-line no-var
  var __nd_react: typeof ReactFirstEval | undefined;
}

function pinnedReact(): typeof ReactFirstEval {
  if (!globalThis.__nd_react) globalThis.__nd_react = ReactFirstEval;
  return globalThis.__nd_react;
}

// Each export wraps a call to pinnedReact() rather than binding the hook
// directly, so the wrapper (recreated fresh every re-eval, same as any other
// function in this module) always dispatches to whatever is CURRENTLY
// stashed in globalThis — the first-eval react instance, every time.
export const useState: typeof ReactFirstEval.useState = (...a) => pinnedReact().useState(...a);
export const useEffect: typeof ReactFirstEval.useEffect = (...a) => pinnedReact().useEffect(...a);
export const useLayoutEffect: typeof ReactFirstEval.useLayoutEffect = (...a) => pinnedReact().useLayoutEffect(...a);
export const useMemo: typeof ReactFirstEval.useMemo = (...a) => pinnedReact().useMemo(...a);
export const useCallback: typeof ReactFirstEval.useCallback = (...a) => pinnedReact().useCallback(...a);
export const useRef: typeof ReactFirstEval.useRef = (...a) => pinnedReact().useRef(...a);
export const useContext: typeof ReactFirstEval.useContext = (...a) => pinnedReact().useContext(...a);
export const useReducer: typeof ReactFirstEval.useReducer = (...a) => pinnedReact().useReducer(...a);
export const useTransition: typeof ReactFirstEval.useTransition = (...a) => pinnedReact().useTransition(...a);
export const useDeferredValue: typeof ReactFirstEval.useDeferredValue = (...a) =>
  pinnedReact().useDeferredValue(...a);
export const useSyncExternalStore: typeof ReactFirstEval.useSyncExternalStore = (...a) =>
  pinnedReact().useSyncExternalStore(...a);
export const useId: typeof ReactFirstEval.useId = (...a) => pinnedReact().useId(...a);
export const use: typeof ReactFirstEval.use = (a) => pinnedReact().use(a);
export const startTransition: typeof ReactFirstEval.startTransition = (...a) =>
  pinnedReact().startTransition(...a);

// memo/forwardRef/createContext/Suspense/Fragment don't touch the dispatcher
// (no hook call inside them), so they need no pinning for correctness — but
// they're re-exported here too so app code can do
// `import { useState, memo, Suspense } from "@nativedesktop/react"` from one
// place instead of splitting imports across two packages.
export const memo: typeof ReactFirstEval.memo = (...a) => pinnedReact().memo(...a);
export const forwardRef: typeof ReactFirstEval.forwardRef = (...a) => pinnedReact().forwardRef(...a);
export const createContext: typeof ReactFirstEval.createContext = (...a) => pinnedReact().createContext(...a);
export const Suspense: typeof ReactFirstEval.Suspense = ReactFirstEval.Suspense;
export const Fragment: typeof ReactFirstEval.Fragment = ReactFirstEval.Fragment;
