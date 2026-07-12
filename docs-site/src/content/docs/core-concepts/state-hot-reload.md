---
title: State & Hot Reload
description: The hooks re-export contract that makes hot reload state-preserving, and React Compiler's opt-in status.
---

**Status: landed**, with one required convention.

## The one rule: import hooks from `@nativedesktop/react`

`ND_DEV=1` runs the Bun child under `bun --hot`, which keeps the same OS process and NDP socket
across an edit — but re-evaluates the **entire** module graph on every edit, `react` and
`react-reconciler` included. A hook imported straight from `react` resolves against a fresh,
re-evaluated module instance whose dispatcher was never attached to any reconciler — that's the
`Invalid hook call: resolveDispatcher().useState` crash this convention exists to avoid.

`packages/react/src/dev-react.ts` stashes the **first-eval** `react` module instance in
`globalThis` and re-exports its hooks through wrapper functions that always dispatch to that
stashed instance. The reconciler created on first boot closes over that same first-eval `react`
instance, so a hook resolved through `@nativedesktop/react` always talks to the dispatcher the live
reconciler actually drives.

```ts
import { useState, useEffect, useMemo } from "@nativedesktop/react"; // correct, every time
```

## What actually preserves state across an edit

`render()`'s hot-reload path doesn't call `updateContainer` again on re-eval — a fresh `<App/>`
element's `.type` is a new function reference every re-eval, which the reconciler would otherwise
treat as a type change and fully remount. Instead it calls `hotUpdateRoot()`, which re-registers the
new component type under the same `react-refresh` family key the first boot established, and asks
`react-refresh` to patch the live fiber tree in place. `react-refresh`'s family/root registries are
also pinned to the first-eval module instance via the same `globalThis` stash — no Bun-level module
aliasing needed, since `react-refresh` has no internal `require()` calls of its own to trip over.

Two alternative mechanisms were tried and rejected before landing on the above (recorded so nobody
re-attempts them): `Bun.plugin`'s module aliasing throws `Requested module is already fetched` the
moment an aliased specifier is touched by both ESM `import` and CJS `require()` — which
`react-reconciler`'s bundled CJS does internally for `"react"`, unavoidably, since every app entry
uses genuine ESM `import`.

## React Compiler: opt-in, working, off by default

**Status: landed (opt-in).** `babel-plugin-react-compiler@1.0.0` runs cleanly as a build pre-pass
and the compiled output runs correctly against `@nativedesktop/react`. It's a pre-pass, not inline,
because Bun's runtime transpiler doesn't run Babel plugins and `bun --hot` re-evaluates modules
through Bun's own transpiler only. The compile step runs two Babel plugins in one pass:
`babel-plugin-react-compiler` (the memoization transform) and `@babel/plugin-transform-react-jsx`
(JSX → `@nativedesktop/react/jsx-runtime` calls, chosen deliberately so the compiled output contains
no JSX syntax left for Bun to pragma-select on).

`bun run dev` (`ND_DEV=1` + `--hot`) still points at uncompiled `src/`, so hot reload and
react-refresh are unaffected by whether the compiler is enabled. Use the compiled path for a
production-style run:

```bash
bun run compile && ND_SCRIPT=dist/main.tsx ./zig-out/bin/nd-hello
```
